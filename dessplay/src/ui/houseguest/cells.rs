//! Panic-free cell writes that keep wide glyphs whole and never touch
//! protocol-image cells.

use tuirealm::ratatui::buffer::{Buffer, Cell, CellDiffOption};
use tuirealm::ratatui::style::{Color, Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A glyph she paints: foreground and modifiers only. The background is
/// always the underlying cell's, and underlying modifiers are cleared so
/// a selection highlight or underline can't bleed into her.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ink {
    pub fg: Color,
    pub modifier: Modifier,
}

impl Ink {
    pub const fn new(fg: Color, modifier: Modifier) -> Self {
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

/// Write one glyph at `(x, y)`. Returns whether it was written:
/// off-buffer and image (`skip`) cells refuse. A wide glyph that would
/// be half-covered is blanked instead, so no frame ever holds half of
/// one (its trailing cell is ratatui's reset blank). A wide `glyph` is
/// one brick too: it takes the cell after it as its second half
/// (blanking what's there, and the rest of any wide glyph that starts
/// there), and is refused where that cell is off the buffer or an image
/// cell.
pub(super) fn put(buf: &mut Buffer, x: i32, y: i32, glyph: char, ink: Ink) -> bool {
    let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
        return false;
    };
    match buf.cell((x, y)) {
        Some(cell) if !untouchable(cell) => {}
        _ => return false,
    }
    let brick = glyph.width().unwrap_or(0) > 1;
    let second = x.checked_add(1).filter(|_| brick);
    if let Some(right) = second {
        match buf.cell((right, y)) {
            Some(cell) if !untouchable(cell) => {}
            _ => return false,
        }
    } else if brick {
        return false;
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
    // What this glyph covers: its cell, and its second half's. A wide
    // glyph starting in either loses its second half too.
    for at in std::iter::once(x).chain(second) {
        let wide = buf.cell((at, y)).is_some_and(|cell| width(cell) > 1);
        if let Some(right) = at.checked_add(1).filter(|_| wide) {
            blank(buf, right, y);
        }
        if Some(at) == second {
            blank(buf, at, y);
        }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use tuirealm::ratatui::layout::Rect;

    const INK: Ink = Ink::new(Color::Reset, Modifier::BOLD);

    /// The row of `buf` as its symbols, one per cell.
    fn row(buf: &Buffer, y: u16) -> Vec<String> {
        (0..buf.area.width)
            .map(|x| buf.cell((x, y)).unwrap().symbol().to_owned())
            .collect()
    }

    /// No cell of `buf` holds half of a wide glyph: the cell after one is
    /// blank.
    fn whole(buf: &Buffer) {
        for y in 0..buf.area.height {
            for x in 0..buf.area.width.saturating_sub(1) {
                if width(buf.cell((x, y)).unwrap()) > 1 {
                    assert_eq!(buf.cell((x + 1, y)).unwrap().symbol(), " ", "({x}, {y})");
                }
            }
        }
    }

    /// A wide glyph is one brick: written, it takes the cell after it as
    /// its second half (blanking what stood there, and the rest of a
    /// wide glyph that started there); it's refused, writing nothing,
    /// where it has no second half (the last column) or that cell is an
    /// image's.
    #[test]
    fn a_wide_glyph_is_put_whole() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 6, 3));
        // Over a narrow glyph in its second half.
        assert!(put(&mut buf, 1, 0, 'n', INK));
        assert!(put(&mut buf, 0, 0, '漢', INK));
        whole(&buf);
        assert_eq!(row(&buf, 0)[..3], ["漢", " ", " "]);
        // Over a wide glyph starting in its second half: that one goes
        // whole (no half of it left at 3).
        assert!(put(&mut buf, 3, 0, '語', INK));
        assert!(put(&mut buf, 2, 0, '漢', INK));
        whole(&buf);
        assert_eq!(row(&buf, 0), ["漢", " ", "漢", " ", " ", " "]);
        // At the last column it has no second half: refused.
        assert!(!put(&mut buf, 5, 1, '漢', INK));
        assert_eq!(row(&buf, 1), [" "; 6]);
        // Its second half on an image's cell: refused, nothing written
        // (not its first cell, nor the image's).
        buf.cell_mut((3, 2))
            .unwrap()
            .set_symbol("x")
            .set_diff_option(CellDiffOption::Skip);
        let before = buf.clone();
        assert!(!put(&mut buf, 2, 2, '漢', INK));
        assert_eq!(buf, before);
    }
}
