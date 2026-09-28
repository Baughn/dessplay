//! Panic-free cell writes that keep wide glyphs whole and never touch
//! protocol-image cells.

use tuirealm::ratatui::buffer::{Buffer, Cell, CellDiffOption};
use tuirealm::ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthStr;

/// A glyph she paints: foreground and modifiers only. The background is
/// always the underlying cell's, and underlying modifiers are cleared so
/// a selection highlight or underline can't bleed into her.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ink {
    pub fg: Color,
    pub modifier: Modifier,
}

impl Ink {
    pub fn new(fg: Color, modifier: Modifier) -> Self {
        Self { fg, modifier }
    }

    fn style(self) -> Style {
        Style::new()
            .fg(self.fg)
            .remove_modifier(Modifier::all())
            .add_modifier(self.modifier)
    }
}

/// Cells carrying protocol-image escapes (or otherwise special to the
/// diff) are never touched: text written there would corrupt the image.
#[allow(deprecated)] // ratatui 0.30 keeps the old flag alongside the option
pub(super) fn untouchable(cell: &Cell) -> bool {
    cell.diff_option != CellDiffOption::None || cell.skip
}

pub(super) fn width(cell: &Cell) -> usize {
    cell.symbol().width()
}

fn blank(buf: &mut Buffer, x: u16, y: u16) {
    if let Some(cell) = buf.cell_mut((x, y))
        && !untouchable(cell)
    {
        cell.set_symbol(" ");
    }
}

/// Write one narrow glyph at `(x, y)`. Returns whether it was written:
/// off-buffer and image (`skip`) cells refuse. A wide glyph that would
/// be half-covered is blanked instead, so no frame ever holds half of
/// one (its trailing cell is ratatui's reset blank).
pub(super) fn put(buf: &mut Buffer, x: i32, y: i32, glyph: char, ink: Ink) -> bool {
    let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
        return false;
    };
    match buf.cell((x, y)) {
        Some(cell) if !untouchable(cell) => {}
        _ => return false,
    }
    if let Some(left) = x.checked_sub(1)
        && let Some(cell) = buf.cell((left, y))
        && width(cell) > 1
    {
        if untouchable(cell) {
            return false;
        }
        blank(buf, left, y);
    }
    let wide = buf.cell((x, y)).is_some_and(|cell| width(cell) > 1);
    if wide && let Some(right) = x.checked_add(1) {
        blank(buf, right, y);
    }
    let Some(cell) = buf.cell_mut((x, y)) else {
        return false;
    };
    cell.set_char(glyph);
    cell.set_style(ink.style());
    true
}

/// Test fixtures: make `buf` look like a real frame, where the cell after
/// a wide glyph is always ratatui's reset blank (overlapping
/// `set_string`s can break that; the real renderer never does).
#[cfg(test)]
pub(super) fn sanitize(buf: &mut Buffer) {
    let area = buf.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right().saturating_sub(1) {
            let wide = buf.cell((x, y)).is_some_and(|cell| width(cell) > 1);
            let broken = buf
                .cell((x + 1, y))
                .is_some_and(|cell| cell.symbol() != " ");
            if wide
                && broken
                && let Some(cell) = buf.cell_mut((x, y))
            {
                cell.set_symbol(" ");
            }
        }
    }
}
