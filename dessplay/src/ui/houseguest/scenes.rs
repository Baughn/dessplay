//! Scenes that touch text: where they're possible (pure functions of the
//! real frame and the terrain) and the layer operations they queue.
//!
//! Her acts never edit the text layer directly; they queue [`LayerOp`]s,
//! and the paint step applies them against the real frame, where every
//! check lives. An operation the frame no longer supports is refused and
//! she notices.

use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::layout::{Position, Rect};

use super::cells::untouchable;
use super::graphics::strokes;
use super::layer::{TextLayer, takeable};
use super::sprite::{HEIGHT, WIDTH};
use super::terrain::Terrain;

/// Box rows (from her top) where her hands meet the box edge: chest
/// height, the way a person pulls a rope.
pub(super) const PULL_ROWS: [i32; 2] = [1, 2];
/// A line must have at least this many glyphs to be worth pulling.
const MIN_GLYPHS: usize = 3;

/// A line she can pull: standing at `x` on the floor at `y`, the line on
/// `row` ends right beside her box's left edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pull {
    pub x: i32,
    pub y: i32,
    pub row: u16,
    /// Columns of the line's glyphs, left to right.
    pub cells: Vec<u16>,
}

impl Pull {
    /// Which of her box rows the line is on (0 = top).
    pub fn box_row(&self) -> u8 {
        (i32::from(self.row) - (self.y - HEIGHT)).clamp(0, HEIGHT - 1) as u8
    }
}

/// A change to the text layer, applied at paint time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LayerOp {
    /// The line's glyphs should sit `offset` cells right of home.
    Pull {
        row: u16,
        cells: Vec<u16>,
        offset: u16,
    },
}

/// Apply `op` against the real frame. Returns false when the frame no
/// longer supports it (whatever did apply stays, validated as usual).
pub(super) fn apply(op: &LayerOp, layer: &mut TextLayer, buf: &Buffer, protected: &[Rect]) -> bool {
    match op {
        LayerOp::Pull { row, cells, offset } => {
            let mut ok = true;
            // Rightmost first: each glyph moves into the cell (or hole)
            // its right neighbour just left.
            for &c in cells.iter().rev() {
                let source = (c, *row);
                let to = (c.saturating_add(*offset), *row);
                let moved = if layer.at_of(source).is_some() {
                    layer.shift(buf, protected, source, to)
                } else {
                    layer.take(buf, protected, source, to)
                };
                ok &= moved;
            }
            ok
        }
    }
}

/// Every line she could pull from where the terrain lets her stand.
pub(super) fn pulls(buf: &Buffer, terrain: &Terrain, protected: &[Rect]) -> Vec<Pull> {
    let half = WIDTH / 2;
    let mut out = Vec::new();
    for platform in &terrain.platforms {
        let y = platform.y;
        for x in platform.x0..=platform.x1 {
            if !terrain.clear(x, y) {
                continue;
            }
            for box_row in PULL_ROWS {
                let row = y - HEIGHT + box_row;
                let end = x - half - 1;
                let (Ok(end), Ok(row)) = (u16::try_from(end), u16::try_from(row)) else {
                    continue;
                };
                if !takeable(buf, protected, (end, row)) {
                    continue;
                }
                if let Some(cells) = line_ending_at(buf, protected, end, row) {
                    out.push(Pull { x, y, row, cells });
                }
            }
        }
    }
    out
}

/// The line on `row` ending at column `end`: every glyph leftwards up to
/// a border line, a protected or image cell, or the screen edge.
fn line_ending_at(buf: &Buffer, protected: &[Rect], end: u16, row: u16) -> Option<Vec<u16>> {
    let mut cells = Vec::new();
    let mut x = end;
    loop {
        let cell = buf.cell((x, row))?;
        let symbol = cell.symbol();
        let border = symbol.chars().next().is_some_and(|c| strokes(c).is_some());
        if border
            || untouchable(cell)
            || protected.iter().any(|r| r.contains(Position::new(x, row)))
        {
            break;
        }
        if !symbol.trim().is_empty() {
            if !takeable(buf, protected, (x, row)) {
                return None;
            }
            cells.push(x);
        }
        let Some(left) = x.checked_sub(1) else {
            break;
        };
        x = left;
    }
    cells.reverse();
    (cells.len() >= MIN_GLYPHS).then_some(cells)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn room(rows: &[&str]) -> Buffer {
        let mut buf = Buffer::with_lines(rows.iter().copied());
        super::super::cells::sanitize(&mut buf);
        buf
    }

    #[test]
    fn a_short_line_beside_her_box_is_pullable() {
        let buf = room(&[
            "│                 ",
            "│kim: hi          ",
            "│                 ",
            "│                 ",
            "└─────────────────",
        ]);
        let terrain = Terrain::read(&buf, &[], true);
        let pulls = pulls(&buf, &terrain, &[]);
        let pull = pulls.iter().find(|p| p.row == 1).expect("a pull on row 1");
        assert_eq!(pull.x, 10, "her box's left edge sits right after 'hi'");
        assert_eq!(pull.cells, vec![1, 2, 3, 4, 6, 7]);
        assert_eq!(pull.box_row(), 1);
    }

    #[test]
    fn a_line_she_would_have_to_stand_on_is_not() {
        let buf = room(&[
            "│                 ",
            "│kim: a long line!",
            "│                 ",
            "│                 ",
            "└─────────────────",
        ]);
        let terrain = Terrain::read(&buf, &[], true);
        assert!(pulls(&buf, &terrain, &[]).iter().all(|p| p.row != 1));
    }

    #[test]
    fn pulling_moves_the_line_through_its_own_holes() {
        let buf = room(&["kim: hi     "]);
        let mut layer = TextLayer::default();
        let cells = vec![0, 1, 2, 3, 5, 6];
        for offset in 1..=3 {
            let op = LayerOp::Pull {
                row: 0,
                cells: cells.clone(),
                offset,
            };
            assert!(apply(&op, &mut layer, &buf, &[]), "heave {offset}");
        }
        let mut frame = buf.clone();
        layer.validate(&frame, &[]);
        layer.paint(&mut frame);
        assert_eq!(frame, room(&["   kim: hi  "]));
    }

    #[test]
    fn the_newest_line_is_never_pullable() {
        let buf = room(&[
            "│                 ",
            "│kim: hi          ",
            "│                 ",
            "│                 ",
            "└─────────────────",
        ]);
        let newest = [Rect::new(1, 1, 17, 1)];
        let terrain = Terrain::read(&buf, &newest, true);
        assert!(pulls(&buf, &terrain, &newest).iter().all(|p| p.row != 1));
    }
}
