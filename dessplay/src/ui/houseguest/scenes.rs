//! Scenes that touch text: where they're possible (pure functions of the
//! real frame and the terrain) and the layer operations they queue.
//!
//! Her acts never edit the text layer directly; they queue [`LayerOp`]s,
//! and the paint step applies them against the real frame, where every
//! check lives. An operation the frame no longer supports is refused and
//! she notices.

use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::layout::{Position, Rect};

use super::cells::{untouchable, width};
use super::door::DoorSpot;
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
    /// The glyphs there (how she knows the line again once it scrolls).
    pub glyphs: String,
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
    /// The two letters, as shown (how she knows them again).
    pub glyphs: String,
}

/// Makeshift furniture she could make: standing at `x` on the floor at
/// `y`, tear the glyphs at `cells` (nearest her first) off the line on
/// `row` beside her box on `side`, reel them in to her hands, crumple
/// them into `piece` — which stands centred under her — for `then`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Build {
    pub x: i32,
    pub y: i32,
    pub row: u16,
    pub side: Side,
    pub cells: Vec<u16>,
    /// The glyphs there (how she knows the line again once it scrolls).
    pub glyphs: String,
    pub piece: super::room::Shown,
    /// What she makes it for.
    pub then: super::room::Use,
}

impl Build {
    /// The column beside her box where her hands are: each glyph she
    /// reels in vanishes into them from here.
    pub fn hand(&self) -> u16 {
        hand(self.x, self.side)
    }

    /// Reeling steps until every glyph is in her hands.
    pub fn steps(&self) -> u16 {
        steps(&self.cells, self.hand())
    }
}

/// The column beside her box, standing at `x`, on `side`: where her
/// hands are as she reels text in.
fn hand(x: i32, side: Side) -> u16 {
    let half = WIDTH / 2;
    let x = match side {
        Side::Left => x - half - 1,
        Side::Right => x + half + 1,
    };
    x.clamp(0, i32::from(u16::MAX)) as u16
}

/// Reeling steps until every glyph at `cells` is in her hands at
/// column `hand`.
fn steps(cells: &[u16], hand: u16) -> u16 {
    cells
        .iter()
        .map(|&c| c.abs_diff(hand) + 1)
        .max()
        .unwrap_or(0)
}

/// Most glyphs she tears off a line to read (phase 5c D5, HG #72): a
/// strip of it, not the line.
pub(super) const STRIP_GLYPHS: usize = 6;
/// The fewest glyphs worth reading.
const STRIP_MIN: usize = 3;
/// Glyphs she always leaves on a line she borrows a strip of.
const STRIP_LEAVES: usize = 2;

impl Pull {
    /// Whether she can borrow a strip of it to read: long enough to tear
    /// [`STRIP_MIN`] glyphs off and leave [`STRIP_LEAVES`].
    pub fn lends(&self) -> bool {
        self.cells.len() >= STRIP_MIN + STRIP_LEAVES
    }

    /// The columns of the strip she borrows, nearest her first: the end
    /// of the line nearest her, up to [`STRIP_GLYPHS`], leaving
    /// [`STRIP_LEAVES`]. The same for the same line (her act keeps the
    /// line, and this is how both she and the guest know the strip).
    pub fn strip(&self) -> Vec<u16> {
        let count = self
            .cells
            .len()
            .saturating_sub(STRIP_LEAVES)
            .min(STRIP_GLYPHS);
        match self.side {
            Side::Right => self.cells.iter().take(count).copied().collect(),
            Side::Left => self.cells.iter().rev().take(count).copied().collect(),
        }
    }

    /// The column beside her box where her hands are, reeling a strip in.
    pub fn hand(&self) -> u16 {
        hand(self.x, self.side)
    }

    /// Reeling steps until the whole strip is in her hands (and as many
    /// to slide it back).
    pub fn strip_steps(&self) -> u16 {
        steps(&self.strip(), self.hand())
    }
}

/// Text she has torn off a line and holds, a hole where it was (see
/// `Osaka::holding`): to make a piece of, or a strip she borrowed to
/// read and slides back. While she holds it, it's hers; the moment she
/// doesn't, the guest puts back whatever of it is still out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Held {
    Build(Build),
    Strip(Pull),
}

impl Held {
    /// The source cells of the glyphs she holds (or is reeling in).
    pub fn sources(&self) -> Vec<(u16, u16)> {
        let (row, cells) = match self {
            Self::Build(build) => (build.row, build.cells.clone()),
            Self::Strip(pull) => (pull.row, pull.strip()),
        };
        cells.into_iter().map(|c| (c, row)).collect()
    }

    /// A cell of the middle of it (where she glances, losing it).
    pub fn middle(&self) -> (i32, i32) {
        let sources = self.sources();
        let (x, y) = sources.get(sources.len() / 2).copied().unwrap_or_default();
        (i32::from(x), i32::from(y))
    }
}

/// Her grip on the text her act is at (see `Osaka::grip`): the line as
/// she found it while she takes hold of it, then every glyph she has out
/// of it. The guest checks it each paint, against the real frame and her
/// text layer: slipped, she's lost her grip (the text changed or
/// scrolled under her), never reeling in, reading or heaving whatever
/// took its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Grip<'a> {
    /// Taking hold, nothing moved yet: the line's glyphs at `cells` on
    /// `row` still read `glyphs`.
    Line {
        row: u16,
        cells: &'a [u16],
        glyphs: &'a str,
    },
    /// In hand: every glyph from these source cells still out of its
    /// line (moved, or torn off).
    InHand(Vec<(u16, u16)>),
}

impl Grip<'_> {
    /// Whether she still has it, the real frame `buf` read and her
    /// `layer` validated against it.
    pub fn holds(&self, buf: &Buffer, layer: &TextLayer) -> bool {
        match self {
            Self::Line { row, cells, glyphs } => glyphs_at(buf, *row, cells) == *glyphs,
            Self::InHand(sources) => layer.holds_all(sources),
        }
    }
}

/// Something she means to do at a spot on some floor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Job {
    Pull(Pull),
    Swap(Swap),
    /// Tear off text to make a piece of furniture of.
    Build(Build),
    /// Use a piece of her furniture.
    Use(super::room::Seat),
    /// Lift a piece of her furniture into her pocket, to move it.
    Lift(Lift),
    /// Set the piece in her pocket down where it's right.
    SetDown(SetDown),
    /// Borrow a strip of a line to read beside where she tore it, and
    /// slide it back (phase 5c D5, HG #72).
    Borrow(Pull),
    /// Go out by her external door, standing at its spot (door batch
    /// D6): `why` says which gap its door opens on.
    Leave {
        spot: DoorSpot,
        why: Leave,
    },
}

/// Why she goes out by her door (door batch D6), so which gap it opens
/// on is never derived from mixed state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Leave {
    /// Her routine: school (the door's gap never ends; the visit ends
    /// once it has closed behind her).
    School,
    /// The stage's school scene (door batch D10): out and back in after
    /// a short gap, the visit never ending.
    Stage,
}

/// Lifting `repair.piece`, standing at `(x, y)` (beside it on its
/// floor) facing `side`, to move it as `repair` says (and, setting about
/// it, the spots she'd try it in after that one: `trials`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Lift {
    pub repair: super::rules::Repair,
    pub trials: super::rules::Trials,
    pub x: i32,
    pub y: i32,
    pub side: Side,
}

/// Setting `piece` down at `to`, standing at `(x, y)` (beside where it
/// goes, on that floor) facing `side`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SetDown {
    pub piece: super::room::Furniture,
    pub to: super::rules::Placement,
    pub x: i32,
    pub y: i32,
    pub side: Side,
}

impl Job {
    /// The job, borrowed.
    pub fn by_ref(&self) -> JobRef<'_> {
        match self {
            Self::Pull(p) => JobRef::Pull(p),
            Self::Swap(s) => JobRef::Swap(s),
            Self::Build(b) => JobRef::Build(b),
            Self::Use(seat) => JobRef::Use(seat),
            Self::Lift(lift) => JobRef::Lift(lift),
            Self::SetDown(set) => JobRef::SetDown(set),
            Self::Borrow(p) => JobRef::Borrow(p),
            Self::Leave { spot, why } => JobRef::Leave(*spot, *why),
        }
    }

    /// Where she stands to do it.
    pub fn spot(&self) -> (i32, i32) {
        self.by_ref().spot()
    }

    /// A step of moving a piece of her home (the piece in her pocket
    /// owns what's lost if it's let go; see `Osaka::drop_heading`).
    pub fn carry(&self) -> bool {
        matches!(self, Self::Lift(_) | Self::SetDown(_))
    }

    pub fn side(&self) -> Side {
        self.by_ref().side()
    }
}

/// The glyphs shown at `cells` of `row`, in that order.
fn glyphs_at(buf: &Buffer, row: u16, cells: &[u16]) -> String {
    cells
        .iter()
        .filter_map(|&c| buf.cell((c, row)).map(|cell| cell.symbol().to_owned()))
        .collect()
}

/// A job, borrowed from wherever it's kept (her act holds the one she's
/// at).
#[derive(Clone, Copy, Debug)]
pub(super) enum JobRef<'a> {
    Pull(&'a Pull),
    Swap(&'a Swap),
    Build(&'a Build),
    Use(&'a super::room::Seat),
    Lift(&'a Lift),
    SetDown(&'a SetDown),
    Borrow(&'a Pull),
    Leave(DoorSpot, Leave),
}

impl JobRef<'_> {
    /// Where she stands to do it.
    pub fn spot(self) -> (i32, i32) {
        match self {
            Self::Pull(p) | Self::Borrow(p) => (p.x, p.y),
            Self::Swap(s) => (s.x, s.y),
            Self::Build(b) => (b.x, b.y),
            Self::Use(seat) => (seat.x, seat.y),
            Self::Lift(l) => (l.x, l.y),
            Self::SetDown(s) => (s.x, s.y),
            Self::Leave(door, _) => door.spot(),
        }
    }

    /// The cells of the text it's about, if it's about text: a line's
    /// glyphs where they stand in it (a borrow's strip, a build's
    /// glyphs), a swap's two letters where they came from and where
    /// they are.
    pub fn text_cells(self) -> Vec<(u16, u16)> {
        let on = |row: u16, cells: Vec<u16>| cells.into_iter().map(|c| (c, row)).collect();
        match self {
            Self::Pull(p) => on(p.row, p.cells.clone()),
            Self::Borrow(p) => on(p.row, p.strip()),
            Self::Build(b) => on(b.row, b.cells.clone()),
            Self::Swap(s) => vec![s.a.source, s.a.at, s.b.source, s.b.at],
            Self::Use(_) | Self::Lift(_) | Self::SetDown(_) => Vec::new(),
            // Never asked: `Act::at_job` yields no Leave (at her door
            // she's at an `Act::Door`), and a door is about no text.
            Self::Leave(..) => Vec::new(),
        }
    }

    pub fn side(self) -> Side {
        match self {
            Self::Pull(p) | Self::Borrow(p) => p.side,
            Self::Swap(s) => s.side,
            Self::Build(b) => b.side,
            Self::Use(seat) => match seat.facing {
                super::sprite::Facing::Left => Side::Left,
                super::sprite::Facing::Right => Side::Right,
            },
            Self::Lift(l) => l.side,
            Self::SetDown(s) => s.side,
            // Facing out, toward her door's wall.
            Self::Leave(door, _) => match door.out() {
                super::sprite::Facing::Left => Side::Left,
                super::sprite::Facing::Right => Side::Right,
            },
        }
    }

    /// Which of her box rows her hands work at (0 = top).
    pub fn box_row(self) -> u8 {
        let (row, y) = match self {
            Self::Pull(p) | Self::Borrow(p) => (p.row, p.y),
            Self::Swap(s) => (s.row, s.y),
            Self::Build(b) => (b.row, b.y),
            Self::Use(_) => return 1,
            // Never asked: `Act::at_job` yields no Leave (at her door
            // she's at an `Act::Door`). It would be the knob.
            Self::Leave(..) => return 1,
            // Bent down to the piece at her feet.
            Self::Lift(_) | Self::SetDown(_) => return (HEIGHT - 1) as u8,
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
    /// Reel the glyphs at `cells` on `row` (nearest her first) `step`
    /// cells towards her hands at column `hand`: each that would pass
    /// them is in her hands instead — torn off, a hole in its line.
    Reel {
        row: u16,
        cells: Vec<u16>,
        hand: u16,
        step: u16,
    },
    /// [`LayerOp::Reel`] run backwards: the glyphs from `cells` on `row`
    /// (nearest her hands first) out of her hands at column `hand`, as
    /// they were `step` cells into reeling them in (0: home, the line
    /// whole). Sliding back a strip she borrowed to read (phase 5c D5):
    /// a step at a time, farthest first, each into the cell its
    /// neighbour leaves.
    Unreel {
        row: u16,
        cells: Vec<u16>,
        hand: u16,
        step: u16,
    },
    /// Make `piece` of the glyphs torn from `cells` on `row`, for
    /// `purpose` (refused unless every one of them is still torn off).
    Make {
        row: u16,
        cells: Vec<u16>,
        piece: super::room::Shown,
        purpose: super::room::Use,
    },
}

impl LayerOp {
    /// Whether a refusal means she lost her grip on something she was
    /// holding (the text changed under her hands).
    pub fn grips(&self) -> bool {
        matches!(
            self,
            Self::Pull { .. } | Self::Reel { .. } | Self::Unreel { .. }
        )
    }

    /// The source cells it's about.
    pub fn sources(&self) -> Vec<(u16, u16)> {
        match self {
            Self::Pull { row, cells, .. }
            | Self::Reel { row, cells, .. }
            | Self::Unreel { row, cells, .. }
            | Self::Make { row, cells, .. } => cells.iter().map(|&c| (c, *row)).collect(),
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
        LayerOp::Make { .. } => layer.torn_intact(&op.sources()),
        LayerOp::Reel {
            row,
            cells,
            hand,
            step,
        } => {
            let mut ok = true;
            // Nearest first: each moves into the cell its neighbour left.
            for &c in cells {
                let source = (c, *row);
                // Past the first step every glyph is already hers (moved
                // or torn off): one that went back to its line under her
                // (it changed, or scrolled) is lost, never taken afresh.
                if *step > 1 && !layer.holds_all(&[source]) {
                    ok = false;
                    continue;
                }
                // Cells until its near edge is at her hands (a wide
                // glyph's second half must never reach her box).
                let wide = buf.cell(source).map_or(1, |cell| width(cell).max(1) as u16);
                let far = if c > *hand {
                    c - *hand
                } else {
                    (*hand + 1).saturating_sub(c + wide)
                };
                if *step > far {
                    ok &= layer.absorb(buf, protected, source);
                    continue;
                }
                let x = if c > *hand { c - step } else { c + step };
                ok &= match (x == c, layer.at_of(source).is_some()) {
                    (true, _) => true,
                    (false, true) => layer.shift(buf, protected, source, (x, *row)),
                    (false, false) => layer.take(buf, protected, source, (x, *row)),
                };
            }
            ok
        }
        LayerOp::Unreel {
            row,
            cells,
            hand,
            step,
        } => {
            let mut ok = true;
            // Farthest first: each moves into the cell its neighbour
            // (farther out) just left.
            for &c in cells.iter().rev() {
                let source = (c, *row);
                // As far as `Reel` reckons it: past this, in her hands.
                let wide = buf.cell(source).map_or(1, |cell| width(cell).max(1) as u16);
                let far = if c > *hand {
                    c - *hand
                } else {
                    (*hand + 1).saturating_sub(c + wide)
                };
                if *step > far {
                    continue;
                }
                let x = if c > *hand { c - step } else { c + step };
                ok &= layer.give_back(buf, protected, source, (x, *row));
            }
            ok
        }
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
                // Past the first cell every glyph is already moved: one
                // that went back to its line under her is lost, never
                // pulled afresh.
                if offset.unsigned_abs() > 1 && layer.at_of(source).is_none() {
                    ok = false;
                    continue;
                }
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
            // She stays a while at a job: only where her box is calm.
            if !terrain.restful(x, y) {
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
                        let glyphs = glyphs_at(buf, row, &cells);
                        out.push(Pull {
                            x,
                            y,
                            row,
                            side,
                            cells,
                            glyphs,
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
            // She stays a while at a job: only where her box is calm.
            if !terrain.restful(x, y) {
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
                                glyphs: glyphs_at(buf, row, &[a.min(b), a.max(b)]),
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

/// Makeshift pieces she could make of `items`, the next being `id`: at
/// a line in `pulls` long enough to tear at least [`scrap::MIN_GLYPHS`]
/// off and leave some, where the piece fits ([`room::fits`], clear by
/// `clear`, with room for her to use it) centred under the spot she
/// tears from — so she stands over it as she crumples. `then` says what
/// she could use the finished piece for there (a sofa may face a TV).
pub(super) fn builds(
    buf: &Buffer,
    pulls: &[Pull],
    items: &[super::room::Furniture],
    id: super::room::MadeId,
    clear: &dyn Fn(i32, i32) -> bool,
    then: &dyn Fn(&super::room::Shown) -> Vec<super::room::Use>,
) -> Vec<Build> {
    use super::room::{Shown, fits, roomy};
    use super::scrap::{self, Scrap};
    use super::sprite::Facing;
    let mut out = Vec::new();
    for pull in pulls
        .iter()
        .filter(|p| p.cells.len() >= scrap::MIN_GLYPHS + 2)
    {
        for &item in items {
            let (cols, _) = scrap::footprint(item);
            for facing in [Facing::Right, Facing::Left] {
                let mut done = Scrap::new(id, &[], 0);
                done.stage = scrap::STAGES;
                let piece = Shown {
                    item,
                    facing,
                    boxed: false,
                    strip: None,
                    left: pull.x - i32::from(cols) / 2,
                    floor: pull.y,
                    scrap: Some(done),
                };
                if !fits(buf, &piece, clear) || !roomy(buf, &piece, clear) {
                    continue;
                }
                let count = (pull.cells.len() - 2).min(scrap::GLYPHS);
                // The end of the line nearest her comes off.
                let cells: Vec<u16> = match pull.side {
                    Side::Right => pull.cells.iter().take(count).copied().collect(),
                    Side::Left => pull.cells.iter().rev().take(count).copied().collect(),
                };
                let glyphs = glyphs_at(buf, pull.row, &cells);
                for what in then(&piece) {
                    out.push(Build {
                        x: pull.x,
                        y: pull.y,
                        row: pull.row,
                        side: pull.side,
                        cells: cells.clone(),
                        glyphs: glyphs.clone(),
                        piece: Shown {
                            scrap: Some(Scrap::new(id, &[], 0)),
                            ..piece
                        },
                        then: what,
                    });
                }
                // One way round is plenty for a spot.
                break;
            }
        }
    }
    out
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
        assert_eq!(JobRef::Pull(pull).box_row(), 1);
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

    /// A borrowed strip is the end of the line nearest her: at most six
    /// glyphs, and at least two left on the line, so a line lends one
    /// only from five glyphs up. On either side of her.
    #[test]
    fn a_strip_is_the_near_end_leaving_two_on_the_line() {
        for side in [Side::Left, Side::Right] {
            for len in 3..12u16 {
                let cells: Vec<u16> = (10..10 + len).collect();
                let pull = Pull {
                    x: 0,
                    y: 0,
                    row: 0,
                    side,
                    cells: cells.clone(),
                    glyphs: String::new(),
                    gap: 0,
                };
                let at = format!("{side:?} len={len}");
                assert_eq!(pull.lends(), len >= 5, "{at}");
                let count = usize::from(len.saturating_sub(2)).min(6);
                let mut strip = pull.strip();
                strip.sort_unstable();
                let near_end = match side {
                    Side::Right => cells[..count].to_vec(),
                    Side::Left => cells[cells.len() - count..].to_vec(),
                };
                assert_eq!(strip, near_end, "{at}");
            }
        }
    }

    /// Sliding a borrowed strip back runs its reel backwards (phase 5c
    /// D5): each step of [`LayerOp::Unreel`] leaves the layer as that
    /// step of [`LayerOp::Reel`] did, the same glyphs out of their line
    /// by the same cells, and its last step puts the line back whole. On
    /// either side of her, with slack and without, wide glyphs among
    /// them.
    #[test]
    fn a_strip_slides_back_the_way_it_came() {
        let rooms = [
            [
                "│                                      ",
                "│kim: hello there                      ",
                "│                                      ",
                "│                                      ",
                "└──────────────────────────────────────",
            ],
            [
                "│                                      ",
                "│                kim: hello there      ",
                "│                                      ",
                "│                                      ",
                "└──────────────────────────────────────",
            ],
            [
                "│                                      ",
                "│kim: hi 日本 xy                       ",
                "│                                      ",
                "│                                      ",
                "└──────────────────────────────────────",
            ],
        ];
        let state = |layer: &TextLayer| {
            let mut entries: Vec<_> = layer.entries().iter().map(|d| (d.source, d.at)).collect();
            entries.sort_unstable();
            let mut holes: Vec<_> = layer.holes().collect();
            holes.sort_unstable();
            (entries, holes)
        };
        let mut seen = Vec::new();
        for rows in rooms {
            let buf = room(&rows);
            let terrain = Terrain::read(&buf, &[], true);
            for pull in pulls(&buf, &terrain, &[]).iter().filter(|p| p.lends()) {
                let at = format!("{rows:?} at {} gap {}", pull.x, pull.gap);
                let (row, cells, hand) = (pull.row, pull.strip(), pull.hand());
                let mut layer = TextLayer::default();
                let mut reeled = vec![state(&layer)];
                for step in 1..=pull.strip_steps() {
                    let op = LayerOp::Reel {
                        row,
                        cells: cells.clone(),
                        hand,
                        step,
                    };
                    assert!(apply(&op, &mut layer, &buf, &[]), "{at}: reel {step}");
                    reeled.push(state(&layer));
                }
                assert!(layer.torn_intact(&cells.iter().map(|&c| (c, row)).collect::<Vec<_>>()));
                for step in (0..pull.strip_steps()).rev() {
                    let op = LayerOp::Unreel {
                        row,
                        cells: cells.clone(),
                        hand,
                        step,
                    };
                    assert!(apply(&op, &mut layer, &buf, &[]), "{at}: unreel {step}");
                    assert_eq!(
                        state(&layer),
                        reeled[usize::from(step)],
                        "{at}: step {step}"
                    );
                }
                assert!(layer.is_empty(), "{at}: whole");
                seen.push((rows[1], pull.side, pull.gap));
            }
        }
        // Each room, each side.
        for (i, side) in [
            (0, Side::Left),
            (1, Side::Right),
            (1, Side::Left),
            (2, Side::Left),
        ] {
            assert!(
                seen.iter().any(|&(r, s, _)| r == rooms[i][1] && s == side),
                "{seen:?}"
            );
        }
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

    /// Text she has in hand (reeling it in, heaving a line) that goes
    /// back to its line under her (it changed, or scrolled: her layer
    /// validated against the new frame) is lost: her next step is
    /// refused, never taking whatever now stands there afresh. With the
    /// frame unchanged, the step goes on.
    #[test]
    fn text_gone_back_under_her_is_never_taken_afresh() {
        let buf = room(&["abcdefgh      "]);
        let changed = room(&["stuvwxyz      "]);
        let row = 0;
        let sources = |cells: &[u16]| cells.iter().map(|&c| (c, row)).collect::<Vec<_>>();
        // Reeling a strip in to her hands at column 9.
        let cells = vec![7, 6, 5, 4, 3, 2];
        let reel = |step| LayerOp::Reel {
            row,
            cells: cells.clone(),
            hand: 9,
            step,
        };
        for change in [false, true] {
            let mut layer = TextLayer::default();
            for step in 1..=2 {
                assert!(apply(&reel(step), &mut layer, &buf, &[]), "reel {step}");
            }
            let frame = if change { &changed } else { &buf };
            layer.validate(frame, &[]);
            assert_eq!(
                apply(&reel(3), &mut layer, frame, &[]),
                !change,
                "reel on, changed: {change}"
            );
            assert_eq!(
                layer.holds_any(&sources(&cells)),
                !change,
                "changed: {change}"
            );
        }
        // Heaving a line on her right towards her.
        let buf = room(&["      hi kim"]);
        let changed = room(&["      yo bob"]);
        let cells = vec![6, 7, 9, 10, 11];
        let pull = |offset| LayerOp::Pull {
            row,
            cells: cells.clone(),
            offset,
        };
        for change in [false, true] {
            let mut layer = TextLayer::default();
            assert!(apply(&pull(-1), &mut layer, &buf, &[]));
            let frame = if change { &changed } else { &buf };
            layer.validate(frame, &[]);
            assert_eq!(
                apply(&pull(-2), &mut layer, frame, &[]),
                !change,
                "heave on, changed: {change}"
            );
            assert_eq!(
                layer.holds_any(&sources(&cells)),
                !change,
                "changed: {change}"
            );
        }
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
