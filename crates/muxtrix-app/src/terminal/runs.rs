//! The terminal run model: how one row of grid cells becomes styled spans.
//!
//! A "run" is a maximal stretch of cells sharing a style. Splitting rows this
//! way is what keeps rendering cheap — one shaped span per run instead of one
//! per cell — and the split is deliberately renderer-agnostic so the same
//! model feeds both the grid and the theme previews.

use crate::theme::Color;
use muxtrix_terminal::GridSnapshot;

use crate::app::{TerminalLink, detected_web_link_ranges, is_valid_web_url};
use crate::themes::TerminalThemePreset;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TerminalRunStyle {
    pub(crate) foreground: muxtrix_terminal::Rgb,
    pub(crate) background: Option<muxtrix_terminal::Rgb>,
    pub(crate) overlay_background: Option<muxtrix_terminal::Rgb>,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) faint: bool,
    pub(crate) underline: bool,
    pub(crate) strikethrough: bool,
    pub(crate) selected: bool,
    pub(crate) link: bool,
    pub(crate) link_hovered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalUnderlineDecoration {
    None,
    Dotted,
    Solid,
}

/// Recognized links own their underline affordance. Terminal applications such
/// as Claude and Codex may already underline a printed URL, but that styling
/// must not make the link look clickable until Ctrl+Shift is held over it.
pub(crate) fn terminal_underline_decoration(
    style: TerminalRunStyle,
) -> TerminalUnderlineDecoration {
    if style.link {
        if style.link_hovered {
            TerminalUnderlineDecoration::Solid
        } else {
            TerminalUnderlineDecoration::Dotted
        }
    } else if style.underline {
        TerminalUnderlineDecoration::Solid
    } else {
        TerminalUnderlineDecoration::None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TerminalStyleRun {
    pub(crate) text: String,
    pub(crate) style: TerminalRunStyle,
    pub(crate) columns: usize,
    pub(crate) kind: TerminalRunKind,
}

/// How a cell may be grouped before Iced lays it out and clips it.
///
/// Variable-width Unicode remains isolated so it cannot move later grid cells.
/// Box-drawing and block-element glyphs are the exception: those characters
/// are explicitly designed to join across fixed terminal cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminalRunKind {
    Ascii,
    /// Adjacent box-drawing characters share one geometry canvas even when
    /// their code points differ. Their arms terminate at the same cell edges.
    BoxDrawing,
    JoinedCellGlyph(char),
    IsolatedUnicode,
}

impl TerminalRunKind {
    pub(crate) fn can_join(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Ascii, Self::Ascii)
                | (Self::BoxDrawing, Self::BoxDrawing)
                | (Self::JoinedCellGlyph(_), Self::JoinedCellGlyph(_))
        ) && self == next
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum TerminalRunGeometry {
    FullBlock,
}

/// Block elements with whole-cell semantics use geometry instead of outlines.
/// Box-drawing runs take the dedicated sprite path before font shaping.
pub(crate) fn terminal_run_geometry(run: &TerminalStyleRun) -> Option<TerminalRunGeometry> {
    match run.kind {
        TerminalRunKind::JoinedCellGlyph('█') => Some(TerminalRunGeometry::FullBlock),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn terminal_style_runs(
    snapshot: &GridSnapshot,
    focused: bool,
    cursor_phase_visible: bool,
    theme: TerminalThemePreset,
) -> Vec<TerminalStyleRun> {
    terminal_row_style_runs(snapshot, focused, cursor_phase_visible, None, theme)
        .into_iter()
        .flatten()
        .collect()
}

pub(crate) fn terminal_row_style_runs(
    snapshot: &GridSnapshot,
    focused: bool,
    cursor_phase_visible: bool,
    hovered_link: Option<&TerminalLink>,
    theme: TerminalThemePreset,
) -> Vec<Vec<TerminalStyleRun>> {
    let mut rows = Vec::with_capacity(snapshot.cells.len());
    for (row_index, cells) in snapshot.cells.iter().enumerate() {
        let ascii_row = cells
            .iter()
            .map(|cell| {
                let bytes = cell.text.as_bytes();
                if cell.columns == 1 && bytes.len() == 1 && bytes[0].is_ascii() {
                    bytes[0]
                } else {
                    b' '
                }
            })
            .collect::<Vec<_>>();
        let detected_links = detected_web_link_ranges(&ascii_row);
        let mut runs = Vec::new();
        // The emulator resolved this row's selected columns for this frame,
        // having already moved the selection with whatever it scrolled.
        let selected_columns = snapshot.selection.get(row_index).copied().flatten();
        for (column_index, cell) in cells.iter().enumerate() {
            let selected = selected_columns.is_some_and(|range| range.contains(column_index));
            let cursor_here = focused
                && snapshot.cursor.is_some_and(|cursor| {
                    cursor.visible
                        && (!cursor.blinking || cursor_phase_visible)
                        && usize::from(cursor.row) == row_index
                        && usize::from(cursor.column) == column_index
                });
            let link_hovered = hovered_link.is_some_and(|link| {
                link.row == snapshot.scrollbar.offset.saturating_add(row_index as u64)
                    && column_index >= link.start_column
                    && column_index < link.end_column
            });
            let link = cell.hyperlink.as_deref().is_some_and(is_valid_web_url)
                || detected_links
                    .iter()
                    .any(|(start, end)| column_index >= *start && column_index < *end);
            let foreground = if cursor_here {
                theme.cursor_text
            } else {
                cell.foreground
            };
            let overlay_background = if selected {
                Some(theme.selection_background)
            } else if cursor_here {
                Some(snapshot.cursor_color.unwrap_or(theme.cursor))
            } else {
                None
            };
            push_terminal_run(
                &mut runs,
                &cell.text,
                usize::from(cell.columns),
                TerminalRunStyle {
                    foreground,
                    background: (cell.background != snapshot.default_background)
                        .then_some(cell.background),
                    overlay_background,
                    bold: cell.bold,
                    italic: cell.italic,
                    faint: cell.faint,
                    underline: cell.underline,
                    strikethrough: cell.strikethrough,
                    selected,
                    link,
                    link_hovered,
                },
            );
        }
        rows.push(runs);
    }
    rows
}

pub(crate) fn push_terminal_run(
    runs: &mut Vec<TerminalStyleRun>,
    text: &str,
    columns: usize,
    style: TerminalRunStyle,
) {
    let kind = terminal_run_kind(text, columns);
    if let Some(run) = runs
        .last_mut()
        .filter(|run| run.style == style && run.kind.can_join(kind))
    {
        run.text.push_str(text);
        run.columns += columns;
    } else {
        runs.push(TerminalStyleRun {
            text: text.to_owned(),
            style,
            columns,
            kind,
        });
    }
}

pub(crate) fn terminal_run_kind(text: &str, columns: usize) -> TerminalRunKind {
    if text.is_ascii() {
        return TerminalRunKind::Ascii;
    }
    if columns == 1
        && let mut characters = text.chars()
    {
        match (characters.next(), characters.next()) {
            (Some('\u{2500}'..='\u{257f}'), None) => return TerminalRunKind::BoxDrawing,
            (Some(character @ '\u{2580}'..='\u{259f}'), None) => {
                return TerminalRunKind::JoinedCellGlyph(character);
            }
            _ => {}
        }
    }
    TerminalRunKind::IsolatedUnicode
}

pub(crate) fn rgb(color: muxtrix_terminal::Rgb) -> Color {
    Color::from_rgb8(color.red, color.green, color.blue)
}
