//! App-owned keyboard input: what [`crate::Message`] carries instead of an
//! `iced::keyboard::Event`.
//!
//! Key handling is application logic — which chord opens the palette, what
//! bytes a key sends to the PTY — so it should not be phrased in a rendering
//! framework's vocabulary. These types mirror the shape iced uses (a named or
//! character key, bitflag modifiers) closely enough that the handler reads the
//! same, but they belong to this crate, and unit tests build them directly.
//!
//! Only the named keys Muxtrix actually binds are modelled. Anything else
//! arrives as [`Key::Unidentified`] and falls through to the terminal, which is
//! the same outcome an unmapped key had before.

use std::fmt;
use std::ops::BitOr;

/// A key that has a name rather than a character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Named {
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    Backspace,
    Delete,
    End,
    Enter,
    Escape,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Home,
    Insert,
    PageDown,
    PageUp,
    Space,
    Tab,
}

/// A logical key press.
///
/// Generic over the string type so a borrowed view (`Key<&str>`) can be matched
/// without cloning the character, mirroring how the handler already reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Key<S = String> {
    Named(Named),
    Character(S),
    /// A key this application does not model. It still reaches the terminal.
    Unidentified,
}

impl Key<String> {
    pub(crate) fn as_ref(&self) -> Key<&str> {
        match self {
            Self::Named(named) => Key::Named(*named),
            Self::Character(character) => Key::Character(character.as_str()),
            Self::Unidentified => Key::Unidentified,
        }
    }
}

/// The modifier keys held during an event.
///
/// A bitflag set rather than four booleans so exact-match tests
/// (`modifiers == Modifiers::COMMAND | Modifiers::SHIFT`) stay expressible —
/// several shortcuts must not fire when an extra modifier is also down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) struct Modifiers(u8);

impl Modifiers {
    pub(crate) const SHIFT: Self = Self(1 << 0);
    pub(crate) const CTRL: Self = Self(1 << 1);
    pub(crate) const ALT: Self = Self(1 << 2);
    pub(crate) const LOGO: Self = Self(1 << 3);

    /// The platform's primary shortcut modifier: Command on macOS, Ctrl
    /// elsewhere.
    pub(crate) const COMMAND: Self = if cfg!(target_os = "macos") {
        Self::LOGO
    } else {
        Self::CTRL
    };

    pub(crate) const fn empty() -> Self {
        Self(0)
    }

    pub(crate) const fn is_empty(self) -> bool {
        self.0 == 0
    }

    const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub(crate) const fn shift(self) -> bool {
        self.contains(Self::SHIFT)
    }

    pub(crate) const fn control(self) -> bool {
        self.contains(Self::CTRL)
    }

    pub(crate) const fn alt(self) -> bool {
        self.contains(Self::ALT)
    }

    pub(crate) const fn logo(self) -> bool {
        self.contains(Self::LOGO)
    }

    pub(crate) const fn command(self) -> bool {
        self.contains(Self::COMMAND)
    }
}

impl BitOr for Modifiers {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.control() {
            parts.push("Ctrl");
        }
        if self.alt() {
            parts.push("Alt");
        }
        if self.shift() {
            parts.push("Shift");
        }
        if self.logo() {
            parts.push("Super");
        }
        formatter.write_str(&parts.join("+"))
    }
}

/// One key press, with everything the handler needs to decide what it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyInput {
    /// The key as engraved, ignoring modifiers — what shortcuts match on.
    pub(crate) key: Key,
    /// The key after the platform applied modifiers and layout. This is what
    /// produces `!` from Shift+1, so it is what character shortcuts compare.
    pub(crate) modified_key: Key,
    pub(crate) modifiers: Modifiers,
    /// The text the key would insert, when it inserts any.
    pub(crate) text: Option<String>,
    pub(crate) repeat: bool,
}

/// A keyboard event, in the three shapes the app reacts to.
///
/// Releases and bare modifier changes carry no key but still update the tracked
/// modifier state, which hover-to-open-link and drag behaviour read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeyEvent {
    Pressed(KeyInput),
    Released { modifiers: Modifiers },
    ModifiersChanged(Modifiers),
}

impl KeyEvent {
    pub(crate) fn modifiers(&self) -> Modifiers {
        match self {
            Self::Pressed(input) => input.modifiers,
            Self::Released { modifiers } | Self::ModifiersChanged(modifiers) => *modifiers,
        }
    }
}

fn named_from_iced(named: iced::keyboard::key::Named) -> Option<Named> {
    use iced::keyboard::key::Named as I;
    Some(match named {
        I::ArrowDown => Named::ArrowDown,
        I::ArrowLeft => Named::ArrowLeft,
        I::ArrowRight => Named::ArrowRight,
        I::ArrowUp => Named::ArrowUp,
        I::Backspace => Named::Backspace,
        I::Delete => Named::Delete,
        I::End => Named::End,
        I::Enter => Named::Enter,
        I::Escape => Named::Escape,
        I::F1 => Named::F1,
        I::F2 => Named::F2,
        I::F3 => Named::F3,
        I::F4 => Named::F4,
        I::F5 => Named::F5,
        I::F6 => Named::F6,
        I::F7 => Named::F7,
        I::F8 => Named::F8,
        I::F9 => Named::F9,
        I::F10 => Named::F10,
        I::F11 => Named::F11,
        I::F12 => Named::F12,
        I::Home => Named::Home,
        I::Insert => Named::Insert,
        I::PageDown => Named::PageDown,
        I::PageUp => Named::PageUp,
        I::Space => Named::Space,
        I::Tab => Named::Tab,
        _ => return None,
    })
}

fn key_from_iced(key: &iced::keyboard::Key) -> Key {
    match key {
        iced::keyboard::Key::Named(named) => {
            named_from_iced(*named).map_or(Key::Unidentified, Key::Named)
        }
        iced::keyboard::Key::Character(character) => Key::Character(character.to_string()),
        iced::keyboard::Key::Unidentified => Key::Unidentified,
    }
}

fn modifiers_from_iced(modifiers: iced::keyboard::Modifiers) -> Modifiers {
    let mut result = Modifiers::empty();
    if modifiers.shift() {
        result = result | Modifiers::SHIFT;
    }
    if modifiers.control() {
        result = result | Modifiers::CTRL;
    }
    if modifiers.alt() {
        result = result | Modifiers::ALT;
    }
    if modifiers.logo() {
        result = result | Modifiers::LOGO;
    }
    result
}

/// Translate an iced keyboard event into the app's own vocabulary.
///
/// This is the only place iced's keyboard types are read; everything
/// downstream works in [`KeyEvent`].
pub(crate) fn from_iced(event: &iced::keyboard::Event) -> KeyEvent {
    match event {
        iced::keyboard::Event::KeyPressed {
            key,
            modified_key,
            modifiers,
            text,
            ..
        } => KeyEvent::Pressed(KeyInput {
            key: key_from_iced(key),
            modified_key: key_from_iced(modified_key),
            modifiers: modifiers_from_iced(*modifiers),
            text: text.as_ref().map(ToString::to_string),
            repeat: false,
        }),
        iced::keyboard::Event::KeyReleased { modifiers, .. } => KeyEvent::Released {
            modifiers: modifiers_from_iced(*modifiers),
        },
        iced::keyboard::Event::ModifiersChanged(modifiers) => {
            KeyEvent::ModifiersChanged(modifiers_from_iced(*modifiers))
        }
    }
}
