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
use super::layer::{Placed, TextLayer, takeable};
use super::sprite::{HEIGHT, WIDTH};
use super::terrain::Terrain;

/// Box rows (from her top) where her hands meet the box edge: chest
/// height, the way a person pulls a rope.
pub(super) const PULL_ROWS: [i32; 2] = [1, 2];
/// A line must have at least this many glyphs to be worth pulling.
const MIN_GLYPHS: usize = 3;
/// Blank cells a line's end may be from her hands: she reels it in
/// before stepping back with it.
pub(super) const PULL_GAP: u16 = 3;
/// Blank cells a word may be from her hands and still be swapped.
const SWAP_GAP: u16 = 2;

/// Which side of her box a line is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Side {
    Left,
    Right,
}

impl Side {
    /// Cells the line moves per heave: away from its side, towards
    /// where she steps.
    pub fn step(self) -> i32 {
        match self {
            Self::Left => 1,
            Self::Right => -1,
        }
    }
}

/// A line she can pull: standing at `x` on the floor at `y`, the line on
/// `row` ends `gap` blank cells beside her box on `side`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Pull {
    pub x: i32,
    pub y: i32,
    pub row: u16,
    pub side: Side,
    /// Columns of the line's glyphs, left to right.
    pub cells: Vec<u16>,
    /// Blank cells between her box and the line's end.
    pub gap: u16,
}

/// Two adjacent letters of a word she could swap: standing at `x` on
/// the floor at `y`, the word on `row` ends right beside her box on
/// `side`, and `a` and `b` (left to right) are the letters to trade, as
/// shown — home, or where she pulled them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Swap {
    pub x: i32,
    pub y: i32,
    pub row: u16,
    pub side: Side,
    pub a: Placed,
    pub b: Placed,
}

/// Something she means to do at a spot on some floor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Job {
    Pull(Pull),
    Swap(Swap),
    /// Use a piece of her furniture.
    Use(super::room::Seat),
}

impl Job {
    /// Where she stands to do it.
    pub fn spot(&self) -> (i32, i32) {
        match self {
            Self::Pull(p) => (p.x, p.y),
            Self::Swap(s) => (s.x, s.y),
            Self::Use(seat) => (seat.x, seat.y),
        }
    }

    pub fn side(&self) -> Side {
        match self {
            Self::Pull(p) => p.side,
            Self::Swap(s) => s.side,
            Self::Use(seat) => match seat.facing {
                super::sprite::Facing::Left => Side::Left,
                super::sprite::Facing::Right => Side::Right,
            },
        }
    }

    /// Which of her box rows her hands work at (0 = top).
    pub fn box_row(&self) -> u8 {
        let (row, y) = match self {
            Self::Pull(p) => (p.row, p.y),
            Self::Swap(s) => (s.row, s.y),
            Self::Use(_) => return 1,
        };
        (i32::from(row) - (y - HEIGHT)).clamp(0, HEIGHT - 1) as u8
    }
}

/// A change to the text layer, applied at paint time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LayerOp {
    /// The line's glyphs should sit `offset` cells from home (positive is
    /// right).
    Pull {
        row: u16,
        cells: Vec<u16>,
        offset: i16,
    },
    /// Trade two letters where they're shown: each moves to the other's
    /// place.
    Swap { a: Placed, b: Placed },
    /// Knock the glyph at `source` loose: it pops up a row and away in
    /// direction `dir` (−1 left, 1 right) — wherever there's room nearest
    /// that — and then falls.
    Knock { source: (u16, u16), dir: i8 },
    /// A knocked glyph drops a row, if there's room below.
    Fall { source: (u16, u16) },
    /// Put these glyphs back where they were shown (home, for most),
    /// together. `tries` counts refusals.
    Restore { to: Vec<Placed>, tries: u8 },
}

impl LayerOp {
    /// Whether a refusal means she lost her grip on something she was
    /// holding (the text changed under her hands).
    pub fn grips(&self) -> bool {
        matches!(self, Self::Pull { .. })
    }

    /// The source cells it's about.
    pub fn sources(&self) -> Vec<(u16, u16)> {
        match self {
            Self::Pull { row, cells, .. } => cells.iter().map(|&c| (c, *row)).collect(),
            Self::Swap { a, b } => vec![a.source, b.source],
            Self::Knock { source, .. } | Self::Fall { source } => vec![*source],
            Self::Restore { to, .. } => to.iter().map(|p| p.source).collect(),
        }
    }
}

/// Apply `op` against the real frame. Returns false when the frame no
/// longer supports it (whatever did apply stays, validated as usual).
pub(super) fn apply(op: &LayerOp, layer: &mut TextLayer, buf: &Buffer, protected: &[Rect]) -> bool {
    match op {
        LayerOp::Swap { a, b } => layer.swap(buf, protected, *a, *b),
        LayerOp::Knock { source, dir } => {
            let (x, y) = *source;
            let dir = i16::from(*dir);
            let up = y.checked_sub(1);
            let spots = [
                (2 * dir, up),
                (dir, up),
                (0, up),
                (2 * dir, Some(y)),
                (dir, Some(y)),
            ];
            spots.into_iter().any(|(dx, row)| {
                let to = x.checked_add_signed(dx).zip(row);
                to.is_some_and(|to| layer.take(buf, protected, *source, to))
            })
        }
        LayerOp::Fall { source } => layer
            .at_of(*source)
            .is_some_and(|(x, y)| layer.shift(buf, protected, *source, (x, y.saturating_add(1)))),
        LayerOp::Restore { to, .. } => layer.place(buf, protected, to),
        LayerOp::Pull { row, cells, offset } => {
            let mut ok = true;
            // Leading glyph first: each moves into the cell (or hole) its
            // neighbour just left.
            let order: Vec<u16> = if *offset > 0 {
                cells.iter().rev().copied().collect()
            } else {
                cells.clone()
            };
            for c in order {
                let source = (c, *row);
                let Some(x) = c.checked_add_signed(*offset) else {
                    ok = false;
                    continue;
                };
                let to = (x, *row);
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
                let Ok(row) = u16::try_from(y - HEIGHT + box_row) else {
                    continue;
                };
                for side in [Side::Left, Side::Right] {
                    let edge = match side {
                        Side::Left => x - half - 1,
                        Side::Right => x + half + 1,
                    };
                    let Ok(edge) = u16::try_from(edge) else {
                        continue;
                    };
                    let dir = -side.step();
                    let Some((start, gap)) = reach(buf, protected, edge, row, dir, PULL_GAP) else {
                        continue;
                    };
                    if let Some(cells) = segment(buf, protected, start, row, dir) {
                        out.push(Pull {
                            x,
                            y,
                            row,
                            side,
                            cells,
                            gap,
                        });
                    }
                }
            }
        }
    }
    out
}

/// Every letter pair she could swap from where the terrain lets her
/// stand: in an ASCII word of three or more letters that ends beside her
/// box, within her reach of its end. `shown` is the frame as she left it
/// (her layer painted over the real one), so the letters of a line she
/// pulled count where they now sit; `protected` must cover the holes
/// she left.
pub(super) fn swaps(
    shown: &Buffer,
    terrain: &Terrain,
    protected: &[Rect],
    layer: &TextLayer,
) -> Vec<Swap> {
    let buf = shown;
    let half = WIDTH / 2;
    let mut out = Vec::new();
    for platform in &terrain.platforms {
        let y = platform.y;
        for x in platform.x0..=platform.x1 {
            if !terrain.clear(x, y) {
                continue;
            }
            for box_row in PULL_ROWS {
                let Ok(row) = u16::try_from(y - HEIGHT + box_row) else {
                    continue;
                };
                for side in [Side::Left, Side::Right] {
                    let edge = match side {
                        Side::Left => x - half - 1,
                        Side::Right => x + half + 1,
                    };
                    let Ok(edge) = u16::try_from(edge) else {
                        continue;
                    };
                    let dir = -side.step();
                    let Some((start, _)) = reach(buf, protected, edge, row, dir, SWAP_GAP) else {
                        continue;
                    };
                    let Some(word) = word(buf, protected, start, row, dir) else {
                        continue;
                    };
                    for pair in word.windows(2).take(REACH) {
                        let &[a, b] = pair else {
                            continue;
                        };
                        let letter =
                            |c: u16| buf.cell((c, row)).map(|cell| cell.symbol().to_owned());
                        if letter(a) != letter(b) {
                            let placed = |c: u16| {
                                let at = (c, row);
                                Placed {
                                    source: layer.shown_from(at).unwrap_or(at),
                                    at,
                                }
                            };
                            out.push(Swap {
                                x,
                                y,
                                row,
                                side,
                                a: placed(a.min(b)),
                                b: placed(a.max(b)),
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

/// The first glyph she can take on `row` from the cell beside her box
/// (`edge`) outwards in `dir`, across at most `max_gap` free cells: its
/// column and the gap.
fn reach(
    buf: &Buffer,
    protected: &[Rect],
    edge: u16,
    row: u16,
    dir: i32,
    max_gap: u16,
) -> Option<(u16, u16)> {
    let mut x = edge;
    for gap in 0..=max_gap {
        if takeable(buf, protected, (x, row)) {
            return Some((x, gap));
        }
        if !super::layer::free(buf, protected, (x, row)) {
            return None;
        }
        x = x.checked_add_signed(dir as i16)?;
    }
    None
}

/// Letter pairs from a word's end she can reach.
const REACH: usize = 2;

/// The ASCII word on `row` starting at `start` and running in `dir`,
/// as columns from `start` outwards — `None` unless `start` holds a
/// letter and the word is whole (bounded by something that isn't a
/// letter or digit) and at least [`MIN_GLYPHS`] long.
fn word(buf: &Buffer, protected: &[Rect], start: u16, row: u16, dir: i32) -> Option<Vec<u16>> {
    let mut cells = Vec::new();
    let mut x = start;
    loop {
        let symbol = buf.cell((x, row)).map(|c| c.symbol().to_owned());
        let letter = symbol
            .as_deref()
            .is_some_and(|s| s.len() == 1 && s.chars().all(|c| c.is_ascii_alphabetic()));
        if !letter || !takeable(buf, protected, (x, row)) {
            // A digit, or a letter she may not touch, means the word
            // goes on past what she could swap: leave it alone.
            let alnum = symbol
                .as_deref()
                .is_some_and(|s| s.chars().any(|c| c.is_alphanumeric()));
            if alnum {
                return None;
            }
            break;
        }
        cells.push(x);
        match x.checked_add_signed(dir as i16) {
            Some(next) => x = next,
            None => break,
        }
    }
    (cells.len() >= MIN_GLYPHS).then_some(cells)
}

/// Glyphs a sneeze at `(x, y)` could knock loose: beside her box (up to
/// three cells out), on her box rows — never a border line, and never
/// anything she mayn't take.
pub(super) fn loose(buf: &Buffer, protected: &[Rect], x: i32, y: i32) -> Vec<(u16, u16)> {
    let half = WIDTH / 2;
    let columns = (x - half - 3..x - half).chain(x + half + 1..=x + half + 3);
    let mut out = Vec::new();
    for column in columns {
        for row in y - HEIGHT..y {
            let (Ok(cx), Ok(cy)) = (u16::try_from(column), u16::try_from(row)) else {
                continue;
            };
            let border = buf
                .cell((cx, cy))
                .and_then(|c| c.symbol().chars().next())
                .is_some_and(|c| strokes(c).is_some());
            if !border && takeable(buf, protected, (cx, cy)) {
                out.push((cx, cy));
            }
        }
    }
    out
}

/// The run of text on `row` from `start` outwards (`dir` −1 = leftwards):
/// glyphs up to a border line, a protected or image cell, the screen edge,
/// or a gap of two blank cells — so a table's columns stay independent
/// while a sentence's single spaces don't split it.
fn segment(buf: &Buffer, protected: &[Rect], start: u16, row: u16, dir: i32) -> Option<Vec<u16>> {
    let mut cells = Vec::new();
    let mut blanks = 0;
    let mut x = start;
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
        // A wide glyph's trailing cell is part of the glyph, not a gap.
        let trailing = x
            .checked_sub(1)
            .and_then(|left| buf.cell((left, row)))
            .is_some_and(|left| super::cells::width(left) > 1);
        if symbol.trim().is_empty() {
            if !trailing {
                blanks += 1;
                if blanks >= 2 {
                    break;
                }
            }
        } else {
            if !takeable(buf, protected, (x, row)) {
                return None;
            }
            blanks = 0;
            cells.push(x);
        }
        match x.checked_add_signed(dir as i16) {
            Some(next) => x = next,
            None => break,
        }
    }
    cells.sort_unstable();
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
        assert_eq!(pull.side, Side::Left);
        assert_eq!(Job::Pull(pull.clone()).box_row(), 1);
    }

    /// A line ending a little way off is still in reach: she reels in
    /// the slack. Further than [`PULL_GAP`] it isn't.
    #[test]
    fn a_line_a_few_cells_off_is_reeled_in() {
        let buf = room(&[
            "│                       ",
            "│kim: hi                ",
            "│                       ",
            "│                       ",
            "└───────────────────────",
        ]);
        let terrain = Terrain::read(&buf, &[], true);
        let pulls = pulls(&buf, &terrain, &[]);
        let gaps: Vec<(i32, u16)> = pulls
            .iter()
            .filter(|p| p.row == 1)
            .map(|p| (p.x, p.gap))
            .collect();
        assert_eq!(gaps, vec![(10, 0), (11, 1), (12, 2), (13, 3)]);
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
                offset: offset as i16,
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

    /// A table row: pulling the right-hand column must not drag the
    /// left-hand one along (the gap between them is two or more cells).
    #[test]
    fn table_columns_are_separate_lines() {
        let buf = room(&["│Futsuka     BQ     ", "│                   "]);
        assert_eq!(segment(&buf, &[], 14, 0, -1), None, "BQ alone is too short");
        let buf = room(&["│Futsuka     BQX    ", "│                   "]);
        assert_eq!(segment(&buf, &[], 15, 0, -1), Some(vec![13, 14, 15]));
        assert_eq!(
            segment(&buf, &[], 7, 0, -1),
            Some(vec![1, 2, 3, 4, 5, 6, 7])
        );
    }

    #[test]
    fn she_swaps_letters_at_the_end_of_a_word_beside_her() {
        let buf = room(&[
            "│                       ",
            "│kim: the cat           ",
            "│                       ",
            "│                       ",
            "└───────────────────────",
        ]);
        let terrain = Terrain::read(&buf, &[], true);
        let swaps = swaps(&buf, &terrain, &[], &TextLayer::default());
        let here: Vec<_> = swaps
            .iter()
            .filter(|s| s.x == 15)
            .map(|s| (s.a.at.0, s.b.at.0))
            .collect();
        assert_eq!(
            here,
            vec![(11, 12), (10, 11)],
            "'at' and 'ca', within reach"
        );
        assert!(swaps.iter().all(|s| s.row == 1 && s.side == Side::Left));
    }

    #[test]
    fn a_word_on_her_right_swaps_from_its_start() {
        let buf = room(&[
            "│                       ",
            "│               dog: hi ",
            "│                       ",
            "│                       ",
            "└───────────────────────",
        ]);
        let terrain = Terrain::read(&buf, &[], true);
        let swaps = swaps(&buf, &terrain, &[], &TextLayer::default());
        let here: Vec<_> = swaps.iter().filter(|s| s.x == 13).collect();
        assert_eq!(here.len(), 2, "{swaps:?}");
        assert!(here.iter().all(|s| s.side == Side::Right));
        assert_eq!((here[0].a.at.0, here[0].b.at.0), (16, 17));
        assert_eq!((here[1].a.at.0, here[1].b.at.0), (17, 18));
    }

    #[test]
    fn only_whole_ascii_words_of_three_letters_are_swapped() {
        for word in ["4cat", "ab", "漢ab", "aaa"] {
            let line = format!("│kim: {word:<18}");
            let blank = format!("│{:23}", "");
            let floor = format!("└{}", "─".repeat(23));
            let buf = room(&[&blank, &line, &blank, &blank, &floor]);
            let terrain = Terrain::read(&buf, &[], true);
            assert_eq!(
                swaps(&buf, &terrain, &[], &TextLayer::default()),
                vec![],
                "{word}"
            );
        }
    }

    #[test]
    fn the_newest_line_is_never_swapped() {
        let buf = room(&[
            "│                       ",
            "│kim: the cat           ",
            "│                       ",
            "│                       ",
            "└───────────────────────",
        ]);
        let newest = [Rect::new(1, 1, 23, 1)];
        let terrain = Terrain::read(&buf, &newest, true);
        assert_eq!(
            swaps(&buf, &terrain, &newest, &TextLayer::default()),
            vec![]
        );
    }

    #[test]
    fn a_sneeze_never_knocks_a_border_loose() {
        let buf = room(&[
            "│  ab     ",
            "│  cd     ",
            "│         ",
            "│         ",
            "└─────────",
        ]);
        let mut loose = loose(&buf, &[], 7, 4);
        loose.sort_unstable();
        assert_eq!(loose, vec![(3, 0), (3, 1), (4, 0), (4, 1)]);
    }

    #[test]
    fn a_line_on_her_right_is_pulled_leftwards() {
        let buf = room(&["      hi kim"]);
        let mut layer = TextLayer::default();
        let cells = vec![6, 7, 9, 10, 11];
        for offset in 1..=2i16 {
            let op = LayerOp::Pull {
                row: 0,
                cells: cells.clone(),
                offset: -offset,
            };
            assert!(apply(&op, &mut layer, &buf, &[]));
        }
        let mut frame = buf.clone();
        layer.validate(&frame, &[]);
        layer.paint(&mut frame);
        assert_eq!(frame, room(&["    hi kim  "]));
    }
}
