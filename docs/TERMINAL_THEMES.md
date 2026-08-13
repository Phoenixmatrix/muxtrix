# Terminal themes

Muxtrix keeps terminal colors separate from the System/Dark/Light appearance
used by application chrome. The terminal theme is selected under **Settings →
Terminal appearance**, previews immediately in Settings, and applies to every
live pane when the user chooses **Apply changes**. Processes and panes do not
restart.

## Color precedence

Themes are applied inside each pane's `libghostty-vt` terminal state. This
preserves the normal terminal-emulator precedence instead of recoloring the
finished frame:

1. Direct RGB colors emitted with SGR `38;2` and `48;2` belong to the terminal
   program and remain unchanged.
2. Runtime OSC 4/10/11/12 overrides belong to the terminal program and remain
   active across a Muxtrix theme change.
3. The selected theme supplies the default foreground, background, cursor, and
   ANSI palette entries 0–15 when the program has not overridden them.
4. Palette entries 16–255 retain Ghostty's built-in xterm-compatible defaults.

Selection foreground/background and cursor text are emulator UI colors and
come from the selected preset. OSC 12 may still override the cursor color.

This matches Ghostty's default-versus-effective color model: setting defaults
preserves active OSC overrides, while Ghostty resolves palette-indexed cells
and direct RGB cells according to their original source.

## Bundled presets

Muxtrix includes Ghostty Default plus fifteen popular presets from Ghostty's
audited theme distribution: TokyoNight, Catppuccin Mocha and Latte, Dracula,
Gruvbox Dark Hard, GitHub Dark and Light Default, Nord, Rose Pine and Rose Pine
Dawn, Kanagawa Wave, iTerm2 Solarized Dark and Light, Atom One Dark, and Monokai
Pro.

The values were taken from Ghostty's `ghostty-themes-release-20260803` resource
bundle. Collection attribution is recorded in `THIRD_PARTY_NOTICES.md`.

## Rendering stability

Ghostty snapshots retain each cell's terminal column width. Muxtrix projects
rows as fixed-height, fixed-column GPU text runs. ASCII cells with identical
style remain coalesced, while Unicode fallback glyphs are isolated to their
assigned columns and wide glyphs explicitly own two columns. This prevents
animated Braille/spinner frames from shifting nearby text and prevents
Unicode-heavy Claude output from exceeding split or unsplit pane boundaries.

