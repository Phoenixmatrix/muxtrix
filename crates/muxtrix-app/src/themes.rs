use std::fmt;

use libghostty_vt::style::Palette;
use muxtrix_terminal::{Rgb, TerminalTheme};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TerminalThemeId {
    /// The application's own preset: terminal background equals the chrome's
    /// panel surface, so panes and chrome read as one material.
    #[default]
    MuxtrixDark,
    Ghostty,
    TokyoNight,
    CatppuccinMocha,
    Dracula,
    GruvboxDarkHard,
    GitHubDarkDefault,
    Nord,
    RosePine,
    KanagawaWave,
    SolarizedDark,
    AtomOneDark,
    MonokaiPro,
    CatppuccinLatte,
    GitHubLightDefault,
    RosePineDawn,
    SolarizedLight,
    TokyonightStorm,
    CatppuccinMacchiato,
    CatppuccinFrappe,
    GruvboxDark,
    EverforestDark,
    Snazzy,
    NightOwl,
    Palenight,
    Zenburn,
    Monokai,
    RosePineMoon,
    KanagawaDragon,
    GitHubDarkDimmed,
    AyuMirage,
    Ubuntu,
    TomorrowNight,
    GruvboxLight,
    OneHalfLight,
    Tomorrow,
}

impl TerminalThemeId {
    pub(crate) const ALL: [Self; 36] = [
        Self::MuxtrixDark,
        Self::Ghostty,
        Self::TokyoNight,
        Self::CatppuccinMocha,
        Self::Dracula,
        Self::GruvboxDarkHard,
        Self::GitHubDarkDefault,
        Self::Nord,
        Self::RosePine,
        Self::KanagawaWave,
        Self::SolarizedDark,
        Self::AtomOneDark,
        Self::MonokaiPro,
        Self::CatppuccinLatte,
        Self::GitHubLightDefault,
        Self::RosePineDawn,
        Self::SolarizedLight,
        Self::TokyonightStorm,
        Self::CatppuccinMacchiato,
        Self::CatppuccinFrappe,
        Self::GruvboxDark,
        Self::EverforestDark,
        Self::Snazzy,
        Self::NightOwl,
        Self::Palenight,
        Self::Zenburn,
        Self::Monokai,
        Self::RosePineMoon,
        Self::KanagawaDragon,
        Self::GitHubDarkDimmed,
        Self::AyuMirage,
        Self::Ubuntu,
        Self::TomorrowNight,
        Self::GruvboxLight,
        Self::OneHalfLight,
        Self::Tomorrow,
    ];

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::MuxtrixDark => "Muxtrix Dark",
            Self::Ghostty => "Ghostty Default",
            Self::TokyoNight => "TokyoNight",
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::Dracula => "Dracula",
            Self::GruvboxDarkHard => "Gruvbox Dark Hard",
            Self::GitHubDarkDefault => "GitHub Dark Default",
            Self::Nord => "Nord",
            Self::RosePine => "Rose Pine",
            Self::KanagawaWave => "Kanagawa Wave",
            Self::SolarizedDark => "iTerm2 Solarized Dark",
            Self::AtomOneDark => "Atom One Dark",
            Self::MonokaiPro => "Monokai Pro",
            Self::CatppuccinLatte => "Catppuccin Latte",
            Self::GitHubLightDefault => "GitHub Light Default",
            Self::RosePineDawn => "Rose Pine Dawn",
            Self::SolarizedLight => "iTerm2 Solarized Light",
            Self::TokyonightStorm => "TokyoNight Storm",
            Self::CatppuccinMacchiato => "Catppuccin Macchiato",
            Self::CatppuccinFrappe => "Catppuccin Frappe",
            Self::GruvboxDark => "Gruvbox Dark",
            Self::EverforestDark => "Everforest Dark",
            Self::Snazzy => "Snazzy",
            Self::NightOwl => "Night Owl",
            Self::Palenight => "Palenight",
            Self::Zenburn => "Zenburn",
            Self::Monokai => "Monokai",
            Self::RosePineMoon => "Rose Pine Moon",
            Self::KanagawaDragon => "Kanagawa Dragon",
            Self::GitHubDarkDimmed => "GitHub Dark Dimmed",
            Self::AyuMirage => "Ayu Mirage",
            Self::Ubuntu => "Ubuntu",
            Self::TomorrowNight => "Tomorrow Night",
            Self::GruvboxLight => "Gruvbox Light",
            Self::OneHalfLight => "One Half Light",
            Self::Tomorrow => "Tomorrow",
        }
    }

    pub(crate) fn preset(self) -> TerminalThemePreset {
        match self {
            Self::MuxtrixDark => muxtrix_dark(),
            Self::Ghostty => ghostty_default(),
            Self::TokyonightStorm => preset(
                self,
                "TokyoNight Storm",
                false,
                0x24283b,
                0xc0caf5,
                0xc0caf5,
                0x24283b,
                0x364a82,
                0xc0caf5,
                [
                    0x1d202f, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
                    0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
                ],
            ),
            Self::CatppuccinMacchiato => preset(
                self,
                "Catppuccin Macchiato",
                false,
                0x24273a,
                0xcad3f5,
                0xcad3f5,
                0x24273a,
                0x5b6078,
                0xcad3f5,
                [
                    0x494d64, 0xed8796, 0xa6da95, 0xeed49f, 0x8aadf4, 0xf5bde6, 0x8bd5ca, 0xb8c0e0,
                    0x5b6078, 0xed8796, 0xa6da95, 0xeed49f, 0x8aadf4, 0xf5bde6, 0x8bd5ca, 0xa5adcb,
                ],
            ),
            Self::CatppuccinFrappe => preset(
                self,
                "Catppuccin Frappe",
                false,
                0x303446,
                0xc6d0f5,
                0xc6d0f5,
                0x303446,
                0x626880,
                0xc6d0f5,
                [
                    0x51576d, 0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xf4b8e4, 0x81c8be, 0xb5bfe2,
                    0x626880, 0xe78284, 0xa6d189, 0xe5c890, 0x8caaee, 0xf4b8e4, 0x81c8be, 0xa5adce,
                ],
            ),
            Self::GruvboxDark => preset(
                self,
                "Gruvbox Dark",
                false,
                0x282828,
                0xebdbb2,
                0xebdbb2,
                0x282828,
                0x504945,
                0xebdbb2,
                [
                    0x282828, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984,
                    0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
                ],
            ),
            Self::EverforestDark => preset(
                self,
                "Everforest Dark",
                false,
                0x2d353b,
                0xd3c6aa,
                0xd3c6aa,
                0x2d353b,
                0x475258,
                0xd3c6aa,
                [
                    0x475258, 0xe67e80, 0xa7c080, 0xdbbc7f, 0x7fbbb3, 0xd699b6, 0x83c092, 0xd3c6aa,
                    0x475258, 0xe67e80, 0xa7c080, 0xdbbc7f, 0x7fbbb3, 0xd699b6, 0x83c092, 0xd3c6aa,
                ],
            ),
            Self::Snazzy => preset(
                self,
                "Snazzy",
                false,
                0x282a36,
                0xeff0eb,
                0xeff0eb,
                0x282a36,
                0x3e404a,
                0xeff0eb,
                [
                    0x000000, 0xff5c57, 0x5af78e, 0xf3f99d, 0x57c7ff, 0xff6ac1, 0x9aedfe, 0xf1f1f0,
                    0x686868, 0xff5c57, 0x5af78e, 0xf3f99d, 0x57c7ff, 0xff6ac1, 0x9aedfe, 0xf1f1f0,
                ],
            ),
            Self::NightOwl => preset(
                self,
                "Night Owl",
                false,
                0x011627,
                0xd6deeb,
                0xd6deeb,
                0x011627,
                0x1d3b53,
                0xd6deeb,
                [
                    0x011627, 0xef5350, 0x22da6e, 0xaddb67, 0x82aaff, 0xc792ea, 0x21c7a8, 0xffffff,
                    0x575656, 0xef5350, 0x22da6e, 0xffeb95, 0x82aaff, 0xc792ea, 0x7fdbca, 0xffffff,
                ],
            ),
            Self::Palenight => preset(
                self,
                "Palenight",
                false,
                0x292d3e,
                0x959dcb,
                0x959dcb,
                0x292d3e,
                0x444267,
                0x959dcb,
                [
                    0x292d3e, 0xf07178, 0xc3e88d, 0xffcb6b, 0x82aaff, 0xc792ea, 0x89ddff, 0x959dcb,
                    0x676e95, 0xf07178, 0xc3e88d, 0xffcb6b, 0x82aaff, 0xc792ea, 0x89ddff, 0xffffff,
                ],
            ),
            Self::Zenburn => preset(
                self,
                "Zenburn",
                false,
                0x3f3f3f,
                0xdcdccc,
                0xdcdccc,
                0x3f3f3f,
                0x2b2b2b,
                0xdcdccc,
                [
                    0x4d4d4d, 0x705050, 0x60b48a, 0xf0dfaf, 0x506070, 0xdc8cc3, 0x8cd0d3, 0xdcdccc,
                    0x709080, 0xdca3a3, 0xc3bf9f, 0xe0cf9f, 0x94bff3, 0xec93d3, 0x93e0e3, 0xffffff,
                ],
            ),
            Self::Monokai => preset(
                self,
                "Monokai",
                false,
                0x272822,
                0xf8f8f2,
                0xf8f8f2,
                0x272822,
                0x49483e,
                0xf8f8f2,
                [
                    0x272822, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf8f8f2,
                    0x75715e, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4, 0xf9f8f5,
                ],
            ),
            Self::RosePineMoon => preset(
                self,
                "Rose Pine Moon",
                false,
                0x232136,
                0xe0def4,
                0xe0def4,
                0x232136,
                0x44415a,
                0xe0def4,
                [
                    0x393552, 0xeb6f92, 0x3e8fb0, 0xf6c177, 0x9ccfd8, 0xc4a7e7, 0xea9a97, 0xe0def4,
                    0x6e6a86, 0xeb6f92, 0x3e8fb0, 0xf6c177, 0x9ccfd8, 0xc4a7e7, 0xea9a97, 0xe0def4,
                ],
            ),
            Self::KanagawaDragon => preset(
                self,
                "Kanagawa Dragon",
                false,
                0x181616,
                0xc5c9c5,
                0xc5c9c5,
                0x181616,
                0x2d4f67,
                0xc5c9c5,
                [
                    0x0d0c0c, 0xc4746e, 0x8a9a7b, 0xc4b28a, 0x8ba4b0, 0xa292a3, 0x8ea4a2, 0xc8c093,
                    0xa6a69c, 0xe46876, 0x87a987, 0xe6c384, 0x7fb4ca, 0x938aa9, 0x7aa89f, 0xc5c9c5,
                ],
            ),
            Self::GitHubDarkDimmed => preset(
                self,
                "GitHub Dark Dimmed",
                false,
                0x22272e,
                0xadbac7,
                0xadbac7,
                0x22272e,
                0x444c56,
                0xadbac7,
                [
                    0x545d68, 0xf47067, 0x57ab5a, 0xc69026, 0x539bf5, 0xb083f0, 0x39c5cf, 0x909dab,
                    0x636e7b, 0xff938a, 0x6bc46d, 0xdaaa3f, 0x6cb6ff, 0xdcbdfb, 0x56d4dd, 0xcdd9e5,
                ],
            ),
            Self::AyuMirage => preset(
                self,
                "Ayu Mirage",
                false,
                0x1f2430,
                0xcbccc6,
                0xcbccc6,
                0x1f2430,
                0x33415e,
                0xcbccc6,
                [
                    0x191e2a, 0xed8274, 0xa6cc70, 0xfad07b, 0x6dcbfa, 0xcfbafa, 0x90e1c6, 0xc7c7c7,
                    0x686868, 0xf28779, 0xbae67e, 0xffd580, 0x73d0ff, 0xd4bfff, 0x95e6cb, 0xffffff,
                ],
            ),
            Self::Ubuntu => preset(
                self,
                "Ubuntu",
                false,
                0x300a24,
                0xeeeeec,
                0xeeeeec,
                0x300a24,
                0x555753,
                0xeeeeec,
                [
                    0x2e3436, 0xcc0000, 0x4e9a06, 0xc4a000, 0x3465a4, 0x75507b, 0x06989a, 0xd3d7cf,
                    0x555753, 0xef2929, 0x8ae234, 0xfce94f, 0x729fcf, 0xad7fa8, 0x34e2e2, 0xeeeeec,
                ],
            ),
            Self::TomorrowNight => preset(
                self,
                "Tomorrow Night",
                false,
                0x1d1f21,
                0xc5c8c6,
                0xc5c8c6,
                0x1d1f21,
                0x373b41,
                0xc5c8c6,
                [
                    0x000000, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6,
                    0x969896, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xffffff,
                ],
            ),
            Self::GruvboxLight => preset(
                self,
                "Gruvbox Light",
                true,
                0xfbf1c7,
                0x3c3836,
                0x3c3836,
                0xfbf1c7,
                0xd5c4a1,
                0x3c3836,
                [
                    0xfbf1c7, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0x7c6f64,
                    0x928374, 0x9d0006, 0x79740e, 0xb57614, 0x076678, 0x8f3f71, 0x427b58, 0x3c3836,
                ],
            ),
            Self::OneHalfLight => preset(
                self,
                "One Half Light",
                true,
                0xfafafa,
                0x383a42,
                0x383a42,
                0xfafafa,
                0xe5e5e6,
                0x383a42,
                [
                    0x383a42, 0xe45649, 0x50a14f, 0xc18401, 0x0184bc, 0xa626a4, 0x0997b3, 0xfafafa,
                    0x4f525e, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xffffff,
                ],
            ),
            Self::Tomorrow => preset(
                self,
                "Tomorrow",
                true,
                0xffffff,
                0x4d4d4c,
                0x4d4d4c,
                0xffffff,
                0xd6d6d6,
                0x4d4d4c,
                [
                    0x000000, 0xc82829, 0x718c00, 0xeab700, 0x4271ae, 0x8959a8, 0x3e999f, 0xffffff,
                    0x8e908c, 0xc82829, 0x718c00, 0xeab700, 0x4271ae, 0x8959a8, 0x3e999f, 0xffffff,
                ],
            ),
            Self::TokyoNight => preset(
                self,
                "TokyoNight",
                false,
                0x1a1b26,
                0xc0caf5,
                0xc0caf5,
                0x15161e,
                0x33467c,
                0xc0caf5,
                [
                    0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6,
                    0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
                ],
            ),
            Self::CatppuccinMocha => preset(
                self,
                "Catppuccin Mocha",
                false,
                0x1e1e2e,
                0xcdd6f4,
                0xf5e0dc,
                0x1e1e2e,
                0xf5e0dc,
                0x1e1e2e,
                [
                    0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de,
                    0x585b70, 0xf7aec2, 0xc2ecbf, 0xfcd682, 0xaeccfc, 0xf398da, 0xb1eae1, 0xa6adc8,
                ],
            ),
            Self::Dracula => preset(
                self,
                "Dracula",
                false,
                0x282a36,
                0xf8f8f2,
                0xf8f8f2,
                0x282a36,
                0x44475a,
                0xffffff,
                [
                    0x21222c, 0xff5555, 0x50fa7b, 0xf1fa8c, 0xbd93f9, 0xff79c6, 0x8be9fd, 0xf8f8f2,
                    0x6272a4, 0xff6e6e, 0x69ff94, 0xffffa5, 0xd6acff, 0xff92df, 0xa4ffff, 0xffffff,
                ],
            ),
            Self::GruvboxDarkHard => preset(
                self,
                "Gruvbox Dark Hard",
                false,
                0x1d2021,
                0xebdbb2,
                0xebdbb2,
                0x1d2021,
                0x665c54,
                0xebdbb2,
                [
                    0x1d2021, 0xcc241d, 0x98971a, 0xd79921, 0x458588, 0xb16286, 0x689d6a, 0xa89984,
                    0x928374, 0xfb4934, 0xb8bb26, 0xfabd2f, 0x83a598, 0xd3869b, 0x8ec07c, 0xebdbb2,
                ],
            ),
            Self::GitHubDarkDefault => preset(
                self,
                "GitHub Dark Default",
                false,
                0x0d1117,
                0xe6edf3,
                0x2f81f7,
                0x6fc1ff,
                0xe6edf3,
                0x0d1117,
                [
                    0x484f58, 0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xb1bac4,
                    0x6e7681, 0xffa198, 0x56d364, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd, 0xffffff,
                ],
            ),
            Self::Nord => preset(
                self,
                "Nord",
                false,
                0x2e3440,
                0xd8dee9,
                0xeceff4,
                0x282828,
                0xeceff4,
                0x4c566a,
                [
                    0x3b4252, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x88c0d0, 0xe5e9f0,
                    0x596377, 0xbf616a, 0xa3be8c, 0xebcb8b, 0x81a1c1, 0xb48ead, 0x8fbcbb, 0xeceff4,
                ],
            ),
            Self::RosePine => preset(
                self,
                "Rose Pine",
                false,
                0x191724,
                0xe0def4,
                0xe0def4,
                0x191724,
                0x403d52,
                0xe0def4,
                [
                    0x26233a, 0xeb6f92, 0x31748f, 0xf6c177, 0x9ccfd8, 0xc4a7e7, 0xebbcba, 0xe0def4,
                    0x6e6a86, 0xeb6f92, 0x31748f, 0xf6c177, 0x9ccfd8, 0xc4a7e7, 0xebbcba, 0xe0def4,
                ],
            ),
            Self::KanagawaWave => preset(
                self,
                "Kanagawa Wave",
                false,
                0x1f1f28,
                0xdcd7ba,
                0xdcd7ba,
                0x1f1f28,
                0xdcd7ba,
                0x1f1f28,
                [
                    0x090618, 0xc34043, 0x76946a, 0xc0a36e, 0x7e9cd8, 0x957fb8, 0x6a9589, 0xc8c093,
                    0x727169, 0xe82424, 0x98bb6c, 0xe6c384, 0x7fb4ca, 0x938aa9, 0x7aa89f, 0xdcd7ba,
                ],
            ),
            Self::SolarizedDark => preset(
                self,
                "iTerm2 Solarized Dark",
                false,
                0x002b36,
                0x839496,
                0x839496,
                0x073642,
                0x073642,
                0x93a1a1,
                [
                    0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xeee8d5,
                    0x335e69, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
                ],
            ),
            Self::AtomOneDark => preset(
                self,
                "Atom One Dark",
                false,
                0x21252b,
                0xabb2bf,
                0xabb2bf,
                0x21252b,
                0x323844,
                0xabb2bf,
                [
                    0x21252b, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
                    0x767676, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xabb2bf,
                ],
            ),
            Self::MonokaiPro => preset(
                self,
                "Monokai Pro",
                false,
                0x2d2a2e,
                0xfcfcfa,
                0xc1c0c0,
                0x8e8d8d,
                0x5b595c,
                0xfcfcfa,
                [
                    0x2d2a2e, 0xff6188, 0xa9dc76, 0xffd866, 0xfc9867, 0xab9df2, 0x78dce8, 0xfcfcfa,
                    0x727072, 0xff6188, 0xa9dc76, 0xffd866, 0xfc9867, 0xab9df2, 0x78dce8, 0xfcfcfa,
                ],
            ),
            Self::CatppuccinLatte => preset(
                self,
                "Catppuccin Latte",
                true,
                0xeff1f5,
                0x4c4f69,
                0xdc8a78,
                0xeff1f5,
                0xdc8a78,
                0xeff1f5,
                [
                    0xbcc0cc, 0xd20f39, 0x40a02b, 0xdf8e1d, 0x1e66f5, 0xea76cb, 0x179299, 0x5c5f77,
                    0xacb0be, 0xe7103f, 0x46b02f, 0xe49931, 0x3878f6, 0xef95d7, 0x19a1a8, 0x6c6f85,
                ],
            ),
            Self::GitHubLightDefault => preset(
                self,
                "GitHub Light Default",
                true,
                0xffffff,
                0x1f2328,
                0x0969da,
                0x3c9cff,
                0x1f2328,
                0xffffff,
                [
                    0x24292f, 0xcf222e, 0x116329, 0x4d2d00, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781,
                    0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f,
                ],
            ),
            Self::RosePineDawn => preset(
                self,
                "Rose Pine Dawn",
                true,
                0xfaf4ed,
                0x575279,
                0x575279,
                0xfaf4ed,
                0xdfdad9,
                0x575279,
                [
                    0xf2e9e1, 0xb4637a, 0x286983, 0xea9d34, 0x56949f, 0x907aa9, 0xd7827e, 0x575279,
                    0x9893a5, 0xb4637a, 0x286983, 0xea9d34, 0x56949f, 0x907aa9, 0xd7827e, 0x575279,
                ],
            ),
            Self::SolarizedLight => preset(
                self,
                "iTerm2 Solarized Light",
                true,
                0xfdf6e3,
                0x657b83,
                0x657b83,
                0xeee8d5,
                0xeee8d5,
                0x586e75,
                [
                    0x073642, 0xdc322f, 0x859900, 0xb58900, 0x268bd2, 0xd33682, 0x2aa198, 0xbbb5a2,
                    0x002b36, 0xcb4b16, 0x586e75, 0x657b83, 0x839496, 0x6c71c4, 0x93a1a1, 0xfdf6e3,
                ],
            ),
        }
    }
}

impl fmt::Display for TerminalThemeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TerminalThemePreset {
    pub(crate) id: TerminalThemeId,
    pub(crate) name: &'static str,
    pub(crate) is_light: bool,
    pub(crate) foreground: Rgb,
    pub(crate) background: Rgb,
    pub(crate) cursor: Rgb,
    pub(crate) cursor_text: Rgb,
    pub(crate) selection_background: Rgb,
    pub(crate) selection_foreground: Rgb,
    pub(crate) ansi: [Rgb; 16],
}

impl TerminalThemePreset {
    pub(crate) const fn terminal_theme(self) -> TerminalTheme {
        TerminalTheme {
            foreground: self.foreground,
            background: self.background,
            cursor: self.cursor,
            ansi: self.ansi,
        }
    }
}

/// The default preset: Ghostty's ANSI palette on the application's own panel
/// surface, so terminal content and chrome share one material instead of the
/// terminal floating as a lighter card.
fn muxtrix_dark() -> TerminalThemePreset {
    let palette = Palette::default();
    TerminalThemePreset {
        id: TerminalThemeId::MuxtrixDark,
        name: "Muxtrix Dark",
        is_light: false,
        background: rgb(0x0d1016),
        foreground: rgb(0xc6cee0),
        cursor: rgb(0xbfcbe4),
        cursor_text: rgb(0x0d1016),
        selection_background: rgb(0x2b3448),
        selection_foreground: rgb(0xe8ecf4),
        ansi: std::array::from_fn(|index| palette.0[index].into()),
    }
}

fn ghostty_default() -> TerminalThemePreset {
    let palette = Palette::default();
    TerminalThemePreset {
        id: TerminalThemeId::Ghostty,
        name: "Ghostty Default",
        is_light: false,
        background: rgb(0x282c34),
        foreground: rgb(0xffffff),
        cursor: rgb(0xffffff),
        cursor_text: rgb(0x282c34),
        selection_background: rgb(0xffffff),
        selection_foreground: rgb(0x282c34),
        ansi: std::array::from_fn(|index| palette.0[index].into()),
    }
}

#[allow(clippy::too_many_arguments)]
fn preset(
    id: TerminalThemeId,
    name: &'static str,
    is_light: bool,
    background: u32,
    foreground: u32,
    cursor: u32,
    cursor_text: u32,
    selection_background: u32,
    selection_foreground: u32,
    ansi: [u32; 16],
) -> TerminalThemePreset {
    TerminalThemePreset {
        id,
        name,
        is_light,
        foreground: rgb(foreground),
        background: rgb(background),
        cursor: rgb(cursor),
        cursor_text: rgb(cursor_text),
        selection_background: rgb(selection_background),
        selection_foreground: rgb(selection_foreground),
        ansi: ansi.map(rgb),
    }
}

const fn rgb(value: u32) -> Rgb {
    Rgb {
        red: ((value >> 16) & 0xff) as u8,
        green: ((value >> 8) & 0xff) as u8,
        blue: (value & 0xff) as u8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_unique_ids_and_complete_ansi_palettes() {
        for (index, id) in TerminalThemeId::ALL.into_iter().enumerate() {
            let preset = id.preset();
            assert_eq!(preset.id, id);
            assert!(!preset.name.is_empty());
            assert!(!TerminalThemeId::ALL[..index].contains(&id));
            assert_ne!(preset.foreground, preset.background);
            assert_eq!(preset.ansi.len(), 16);
        }
    }

    #[test]
    fn bundled_collection_contains_dark_and_light_ghostty_presets() {
        let light = TerminalThemeId::ALL
            .into_iter()
            .filter(|id| id.preset().is_light)
            .count();
        assert!(light >= 4);
        assert!(TerminalThemeId::ALL.len() - light >= 10);
    }
}
