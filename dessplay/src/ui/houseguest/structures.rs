//! Her structures (phase 6): what she puts up in her home with the UI's
//! own line glyphs. In 6a, one shelf on a pane's back wall
//! (`Structure::Shelf`), saved in her record; the frame's cells it's
//! drawn in are what `Written` holds (phase 6a D4: [`read_ground`]
//! writes them once a paint, before anything reads the frame, and the
//! arm of the paint that ends the frame keeps them, as her home's
//! bottom layer, or undoes them).

use serde::{Deserialize, Serialize};
use tuirealm::ratatui::buffer::{Buffer, Cell};
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::{Color, Modifier};

use super::cells::{Ink, put, untouchable, width};
use super::dissolve::Frozen;
use super::room::{Anchor, Extent, Home, Nook, Plan, Shown, Side, Strip, pane_raw};

/// A shelf's run: `┬────────┬`, its brackets at both ends.
pub(super) const SHELF_WIDTH: i32 = 10;

/// The blank cells a shelf keeps past each end of its run, on its row
/// (the user's brackets answer: it reads as a shelf, not as the UI's own
/// rule running on).
pub(super) const MARGIN: i32 = 2;

/// The stages a shelf goes up in: the half nearer its wall, then the
/// rest.
pub(super) const SHELF_STAGES: u8 = 2;

/// The lowest rise of a high shelf: the first row above every hung
/// piece (above the poster's and the clock's, hung 4 and 2 rows tall),
/// so none ever meets it. Derived from the `Spec` table
/// (`rises_clear_every_hung_and_standing_piece`).
#[cfg(test)] // Read from step 4 (D3a: a low shelf's block, under it).
pub(super) const HIGH_RISE: u16 = 7;

/// The rise of a low shelf, the lowest any shelf hangs: just over her
/// head and every piece that stands (none is taller than she is).
pub(super) const LOW_RISE: u16 = 5;

/// The rows a shelf needs clear over its run, at least: the clock's
/// height (and the gremlin's).
pub(super) const LOW_ABOVE: u16 = 2;

/// How far in from either wall of its pane a shelf's run starts, at
/// least: a body or a door's space (`max(WIDTH, SPACE)`), and the 2
/// cells of its margin, so no door's space ever meets it.
pub(super) const EDGE: i32 = 8;

/// Something she has put up with the UI's own lines (phase 6). 6b adds
/// the ladder, 6c the partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) enum Structure {
    Shelf(Shelf),
}

/// A shelf on `nook`'s back wall: `SHELF_WIDTH` cells from `anchor` (its
/// near end, counted from that wall of the pane's raw strip), `rise`
/// rows above the pane's floor (which says high or low for good);
/// `built` of [`SHELF_STAGES`] stages up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct Shelf {
    pub nook: Nook,
    pub anchor: Anchor,
    pub rise: u16,
    pub built: u8,
}

impl Shelf {
    /// Whether this build can take it as read: no more than all its
    /// stages up, and no lower than a low shelf hangs (a lower one would
    /// meet her head, or what stands).
    pub fn sound(&self) -> bool {
        self.built <= SHELF_STAGES && self.rise >= LOW_RISE
    }

    /// Whether all its stages are up: only a finished shelf holds
    /// anything up (phase 6a D6: only a finished shelf is a strip), so a
    /// piece the record puts on one that isn't lands on its pane's floor
    /// (D2).
    pub fn finished(&self) -> bool {
        self.built == SHELF_STAGES
    }

    /// Where its run hangs on its pane's raw strip `host` this frame
    /// (D3): `SHELF_WIDTH` cells from its anchor, `rise` rows above the
    /// floor. `None` where it can't hang this frame: its run or margin
    /// within a body or a door's space of either wall (`EDGE`), or no
    /// `LOW_ABOVE` rows inside the pane over it. Pure geometry: no door,
    /// no text.
    pub fn run(&self, host: Extent) -> Option<Run> {
        let offset = i32::from(self.anchor.offset);
        let from = match self.anchor.side {
            Side::Left => host.from + offset,
            Side::Right => host.to - offset - SHELF_WIDTH,
        };
        let to = from + SHELF_WIDTH;
        let fits = from >= host.from + EDGE
            && to <= host.to - EDGE
            && self.rise >= LOW_RISE
            && host.rows >= self.rise.saturating_add(LOW_ABOVE);
        fits.then_some(Run {
            from,
            to,
            y: host.floor - i32::from(self.rise),
        })
    }

    /// The cells of `run` up so far, each with its glyph: none before
    /// its first stage; then the bracket at its wall's end and the half
    /// of the run nearer that wall (`┬────`); then all of it
    /// (`┬────────┬`).
    pub fn built_on(&self, run: Run) -> Vec<(i32, char)> {
        let half = SHELF_WIDTH / 2;
        let columns = match (self.built, self.anchor.side) {
            (0, _) => run.from..run.from,
            (1, Side::Left) => run.from..run.from + half,
            (1, Side::Right) => run.to - half..run.to,
            _ => run.from..run.to,
        };
        columns
            .map(|x| {
                let bracket = x == run.from || x == run.to - 1;
                (x, if bracket { '┬' } else { '─' })
            })
            .collect()
    }
}

/// A shelf's run this frame: columns `from..to` on row `y`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Run {
    pub from: i32,
    pub to: i32,
    pub y: i32,
}

impl Run {
    /// The run with its margin: the cells nothing else of hers takes
    /// (D4's keep-out), whatever of it is built or shown.
    pub fn kept(self) -> Rect {
        let from = (self.from - MARGIN).max(0);
        let to = (self.to + MARGIN).max(from);
        let (Ok(x), Ok(y), Ok(width)) = (
            u16::try_from(from),
            u16::try_from(self.y),
            u16::try_from(to - from),
        ) else {
            return Rect::default();
        };
        Rect::new(x, y, width, 1)
    }
}

/// Every shelf of `home`'s whose geometry holds on `nooks` this frame,
/// with its pane's raw strip and its run. Pure geometry, read on every
/// frame with a shelf (and by every [`super::door::Keep`]): nothing is
/// allocated for it.
fn hung<'a>(
    home: &'a Home,
    nooks: &'a [(Nook, Rect)],
) -> impl Iterator<Item = (Shelf, Extent, Run)> + 'a {
    home.structures.iter().filter_map(move |&structure| {
        let Structure::Shelf(shelf) = structure;
        let host = pane_raw(nooks.iter().find(|(nook, _)| *nook == shelf.nook)?.1);
        Some((shelf, host, shelf.run(host)?))
    })
}

/// The whole placed run and margin of every shelf whose geometry holds
/// on `nooks`, written this frame or not (an unbuilt half, a run hidden
/// by text): no text she moves or swaps and no piece she makes goes
/// there (D4's keep-out).
pub(super) fn kept_out(home: &Home, nooks: &[(Nook, Rect)]) -> Vec<Rect> {
    hung(home, nooks).map(|(.., run)| run.kept()).collect()
}

/// What of hers a structure keeps off this frame, beyond the plan's chat
/// pane and her pieces as laid out (see [`free`]): the gated view's
/// protected rectangles (a focused pane in use among them), her layer's
/// cells before it's validated (moved text, its sources, torn cells),
/// and what she has made.
pub(super) struct KeepOff<'a> {
    pub protected: &'a [Rect],
    pub layer: &'a [(u16, u16)],
    pub made: &'a [Rect],
}

/// Whether a structure may be written at `(x, y)` (D4's per-cell test):
/// on screen; blank, not the second half of a wide glyph, not an image
/// cell; unstyled, as its pane is (`bg` its pane's background: a blank
/// in reverse video, as a list's selection bar past its entry's text, or
/// in another colour, is as good as text: the user's fifth round);
/// outside `keep_off` and `plan`'s chat; and in no cover of `laid`.
pub(super) fn free(
    buf: &Buffer,
    (x, y): (i32, i32),
    plan: Plan,
    keep_off: &KeepOff,
    laid: &[Rect],
    bg: Color,
) -> bool {
    let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(y)) else {
        return false;
    };
    let Some(cell) = buf.cell((ux, uy)) else {
        return false;
    };
    let trailer = ux
        .checked_sub(1)
        .and_then(|left| buf.cell((left, uy)))
        .is_some_and(|left| width(left) > 1);
    let at = (ux, uy).into();
    cell.symbol().trim().is_empty()
        && !cell.modifier.contains(Modifier::REVERSED)
        && cell.bg == bg
        && !trailer
        && !untouchable(cell)
        && !keep_off.protected.iter().any(|r| r.contains(at))
        && !plan.chat.contains(at)
        && !keep_off.layer.contains(&(ux, uy))
        && !keep_off.made.iter().any(|r| r.contains(at))
        && !laid.iter().any(|r| r.contains(at))
}

/// Write her structures into the frame (D4): each shelf whose geometry
/// holds on `plan`'s panes and whose built run and margin are all
/// [`free`], all of it or none; in its host pane's border ink (the fg of
/// the pane's floor under the run's middle, as her door's wall glyphs
/// take the wall's). Once a paint, before anything reads the frame;
/// the arm of the paint that ends the frame keeps what it wrote or
/// undoes it ([`Written`]). With no structures, nothing is read.
pub(super) fn read_ground(
    buf: &mut Buffer,
    home: &Home,
    plan: Plan,
    keep_off: &KeepOff,
) -> Written {
    let shelves: Vec<_> = hung(home, plan.nooks).collect();
    if shelves.is_empty() {
        return Written::none();
    }
    let laid = home.laid_and_shifted(plan.nooks).shown;
    let mut written = Written::none();
    for (shelf, host, run) in shelves {
        // What stands on this shelf stands on its cells; every other
        // piece's cover is kept clear of it.
        let covers: Vec<Rect> = laid
            .iter()
            .filter(|s| s.strip != Some(Strip::Shelf(shelf.nook)))
            .map(Shown::cover)
            .collect();
        let cells = shelf.built_on(run);
        let (Some(&(first, _)), Some(&(last, _))) = (cells.first(), cells.last()) else {
            continue;
        };
        // Its pane's border under its middle: the ink it's drawn in, and
        // the background a blank of its pane has.
        let border = u16::try_from(run.from + SHELF_WIDTH / 2)
            .ok()
            .zip(u16::try_from(host.floor).ok())
            .and_then(|at| buf.cell(at));
        let bg = border.map_or(Color::Reset, |cell| cell.bg);
        if !(first - MARGIN..=last + MARGIN)
            .all(|x| free(buf, (x, run.y), plan, keep_off, &covers, bg))
        {
            continue;
        }
        let ink = Ink::new(
            border.map_or(Color::Reset, |cell| cell.fg),
            Modifier::empty(),
        );
        for (x, glyph) in cells {
            let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(run.y)) else {
                continue;
            };
            let Some(under) = buf.cell((ux, uy)).cloned() else {
                continue;
            };
            if put(buf, x, run.y, glyph, ink) {
                written.cells.push(Frozen {
                    x: ux,
                    y: uy,
                    glyph,
                    ink,
                    under,
                    face: None,
                    burst: false,
                });
            }
        }
        written.shown_on.push(shelf.nook);
    }
    written
}

/// What her structures wrote into the frame this paint: each cell, with
/// the real cell it covers, and the panes whose shelves stand. Every
/// terrain read and every `under` a cell of hers records takes it (the
/// layout's text checks take it from step 3, D6). The arm of the paint
/// that ends the frame consumes it: [`Written::keep`] into what her home
/// painted (visiting, or standing empty), else [`Written::undo`] before
/// it reads the frame (her goodbye's frame shows nothing of her
/// structures). Dropped holding a cell, it panics in debug builds: a
/// path that forgets it is a test failure, not a stray glyph.
#[must_use = "an arm keeps it (into its painted list) or undoes it"]
#[derive(Debug, Default)]
pub(super) struct Written {
    cells: Vec<Frozen>,
    shown_on: Vec<Nook>,
}

/// Nothing written, for as long as a test needs it (a frame it builds
/// and returns).
#[cfg(test)]
pub(super) static NOTHING: Written = Written {
    cells: Vec::new(),
    shown_on: Vec::new(),
};

impl Written {
    /// Nothing written: the frame is the app's own.
    pub fn none() -> Self {
        Self::default()
    }

    /// The frame as her layout reads it: `buf`, which this was written
    /// into ([`Drawn`]).
    pub fn over<'a>(&'a self, buf: &'a Buffer) -> Drawn<'a> {
        Drawn { buf, written: self }
    }

    /// Cells written by hand (tests of what reads through it).
    #[cfg(test)]
    pub fn of(cells: Vec<Frozen>) -> Self {
        Self {
            cells,
            shown_on: Vec::new(),
        }
    }

    /// Whether a structure was written at `(x, y)`.
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.cells
            .iter()
            .any(|c| i32::from(c.x) == x && i32::from(c.y) == y)
    }

    /// Whether a structure was written in `rect`.
    pub fn meets(&self, rect: Rect) -> bool {
        self.cells.iter().any(|c| rect.contains((c.x, c.y).into()))
    }

    /// What the app drew at `(x, y)`, under anything written there: what
    /// a cell of hers drawn over it records as its `under`.
    pub fn real(&self, buf: &Buffer, x: u16, y: u16) -> Option<Cell> {
        match self.cells.iter().find(|c| (c.x, c.y) == (x, y)) {
            Some(written) => Some(written.under.clone()),
            None => buf.cell((x, y)).cloned(),
        }
    }

    /// The panes whose shelves were written (for the log).
    pub fn shown_on(&self) -> &[Nook] {
        &self.shown_on
    }

    /// Keep the cells into `painted`, first: the bottom layer, whose
    /// `under` is the real frame's (a dissolve stacks what's over them).
    pub fn keep(mut self, painted: &mut Vec<Frozen>) {
        let mut cells = std::mem::take(&mut self.cells);
        cells.append(painted);
        *painted = cells;
    }

    /// Put the real cells back: the frame is the app's own again.
    pub fn undo(mut self, buf: &mut Buffer) {
        for cell in std::mem::take(&mut self.cells).into_iter().rev() {
            if let Some(at) = buf.cell_mut((cell.x, cell.y)) {
                *at = cell.under;
            }
        }
    }
}

/// The frame as her layout reads it (phase 6a D6): the buffer, with what
/// her structures wrote into it this paint. Every layout read that judges
/// the frame's cells (`room::fits` and all built on it) takes one, so a
/// buffer is never read apart from what was written into it: a piece on
/// a shelf stands only on its written cells, and no other piece on one.
/// Made only by [`Written::over`] (in production, the paint's own
/// `Written` over its buffer; a test's frame with none, [`Drawn::bare`]).
#[derive(Clone, Copy)]
pub(super) struct Drawn<'a> {
    buf: &'a Buffer,
    written: &'a Written,
}

impl<'a> Drawn<'a> {
    /// The frame's cells, her structures' included.
    pub fn buf(self) -> &'a Buffer {
        self.buf
    }

    /// What her structures wrote into it.
    pub fn written(self) -> &'a Written {
        self.written
    }

    /// A frame with nothing of hers written into it (tests).
    #[cfg(test)]
    pub fn bare(buf: &'a Buffer) -> Self {
        NOTHING.over(buf)
    }
}

impl Drop for Written {
    fn drop(&mut self) {
        debug_assert!(
            self.cells.is_empty() || std::thread::panicking(),
            "her structures' cells neither kept nor undone"
        );
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::room::{Furniture, Lane, Prop, SPACE};
    use super::super::scrap::{self, MAKES};
    use super::super::sprite::{Facing, HEIGHT, WIDTH};
    use super::*;

    /// The rises are what the `Spec` table and her height make them: a
    /// high shelf hangs over every hung piece's top row, a low one over
    /// her head and every piece that stands (and her window, the lowest
    /// hung piece); every shelf has room over it for the smallest small
    /// decor; and a run starts a body or a door's space, and its margin,
    /// in from either wall. Computed from `Furniture::ALL`, so a new kind
    /// that would meet a shelf breaks it.
    #[test]
    fn rises_clear_every_hung_and_standing_piece() {
        let rows = |item: Furniture| item.spec().footprint.1;
        let hung_top = Furniture::ALL
            .iter()
            .filter_map(|&item| item.spec().hang.map(|hang| hang + rows(item)))
            .max()
            .expect("a hung piece");
        assert_eq!(HIGH_RISE, hung_top + 1);
        let tallest = Furniture::ALL
            .iter()
            .filter(|item| item.spec().hang.is_none())
            .map(|&item| rows(item))
            .max()
            .expect("a standing piece");
        let her = u16::try_from(HEIGHT).unwrap();
        assert_eq!(LOW_RISE, her.max(tallest) + 1);
        // Every piece as the layout lanes it, boxed or not (a parcel
        // stands, whatever its kind: lift 0), and every makeshift one
        // (which stands too): a hung piece's top row is under a high
        // shelf's row, and what stands is under a low one's. A guard: a
        // parcel or a made piece taller than a low shelf's rise breaks
        // it.
        for item in Furniture::ALL {
            for boxed in [false, true] {
                let piece = Prop {
                    boxed,
                    ..Prop::new(item, Nook::Users, 500, Facing::Right)
                };
                match (piece.lane(), item.spec().hang) {
                    (Lane::Wall, Some(hang)) => {
                        assert!(!boxed, "{item:?}: a parcel hangs");
                        assert!(hang + rows(item) < HIGH_RISE, "{item:?}");
                    }
                    (Lane::Floor, _) => {
                        assert!(boxed || item.spec().hang.is_none(), "{item:?}");
                        assert!(rows(item) < LOW_RISE, "{item:?} boxed={boxed}");
                    }
                    (Lane::Wall, None) => panic!("{item:?} hangs with no hang"),
                }
            }
        }
        for item in MAKES {
            assert!(scrap::footprint(item).1 < LOW_RISE, "made {item:?}");
        }
        // Her window hangs under a low shelf too (the wall lane's pieces,
        // the poster and the clock, pack beside one: D3a).
        let window = Furniture::Window;
        assert!(window.spec().hang.unwrap() + rows(window) < LOW_RISE);
        let smallest = Furniture::ALL
            .iter()
            .filter(|item| item.spec().shelf)
            .map(|&item| rows(item))
            .min()
            .expect("small decor");
        assert_eq!(LOW_ABOVE, smallest);
        assert_eq!(EDGE, WIDTH.max(SPACE) + 2);
        assert_eq!(SHELF_STAGES, 2);
        assert_eq!(SHELF_WIDTH, 10);
    }
}

/// `read_ground` against an independent oracle (phase 6a step 2b): the
/// geometry from the `Shelf` record and the nook rect alone, the cell
/// test written out afresh.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod ground {
    use proptest::prelude::*;
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestRng, TestRunner};
    use tuirealm::ratatui::style::Style;
    use tuirealm::ratatui::widgets::{Block, Widget};

    use super::super::cells::untouchable as image_cell;
    use super::super::room::{Furniture, Prop};
    use super::super::sprite::Facing;
    use super::*;

    const NOOKS: [Nook; 3] = [Nook::List, Nook::Users, Nook::Playlist];
    /// The kinds laid out under the shelves (this strategy's own list).
    const KINDS: [Furniture; 6] = [
        Furniture::Sofa,
        Furniture::Tv,
        Furniture::Lamp,
        Furniture::Plant,
        Furniture::Poster,
        Furniture::Clock,
    ];
    const INKS: [Color; 4] = [
        Color::Reset,
        Color::Cyan,
        Color::Rgb(90, 90, 200),
        Color::Yellow,
    ];

    /// A shelf drawn for a pane (when the pane allows one at all, else
    /// anything goes): `how` 0, anything goes; 1, placed by construction
    /// inside where its geometry holds on that pane; 2, at its geometry's
    /// edges, either side of each (an offset of `EDGE - 1` or `EDGE`, the
    /// largest that holds or one past it; a rise of `LOW_RISE - 1` or
    /// `LOW_RISE`, the largest leaving `LOW_ABOVE` rows over it or one
    /// past). `crowd` puts something at the edge of its built run's
    /// margin: 2, a wide glyph just outside it, its second half on the
    /// margin's first cell; 3, an image's (blank) cell on a cell of the
    /// run or its margin, chosen by `pick`; else nothing. (Only the
    /// hand-made draws below use 4, a selection bar in reverse video
    /// across the pane's whole row; 5, a blank in a colour unlike its
    /// pane's on the cell `pick` chooses; 6, the whole pane, border and
    /// inside, in a theme's background of its own; and 7, that, with the
    /// cell `pick` chooses of its built run or margin back in the
    /// terminal's: the strategy never makes them, so its value space is
    /// unchanged.)
    #[derive(Clone, Debug)]
    struct ShelfDraw {
        how: u8,
        right: bool,
        offset: u16,
        rise: u16,
        built: u8,
        crowd: u8,
        pick: u16,
    }

    #[derive(Clone, Debug)]
    struct Draw {
        width: u16,
        height: u16,
        /// Each pane's top, bottom (exclusive), its border's ink, and its
        /// shelf.
        panes: Vec<(u16, u16, usize, Option<ShelfDraw>)>,
        /// `(x, y, text, wide)`.
        text: Vec<(u16, u16, u16, bool)>,
        images: Vec<(u16, u16)>,
        protected: Vec<(u16, u16, u16, u16)>,
        chat: Option<(u16, u16, u16, u16)>,
        layer: Vec<(u16, u16)>,
        made: Vec<(u16, u16, u16, u16)>,
        pieces: Vec<(usize, usize, u16)>,
    }

    fn shelf_draw() -> impl Strategy<Value = ShelfDraw> {
        (
            0u8..3,
            any::<bool>(),
            0u16..80,
            0u16..40,
            0u8..=2,
            0u8..4,
            any::<u16>(),
        )
            .prop_map(|(how, right, offset, rise, built, crowd, pick)| ShelfDraw {
                how,
                right,
                offset,
                rise,
                built,
                crowd,
                pick,
            })
    }

    fn rect_draw(width: u16, height: u16) -> impl Strategy<Value = (u16, u16, u16, u16)> {
        (0..width, 0..height, 1u16..30, 1u16..8)
    }

    fn draw() -> impl Strategy<Value = Draw> {
        (60u16..=140, 12u16..=40, 1usize..=3).prop_flat_map(|(width, height, n)| {
            let pane = (
                0..height / 3,
                0u16..=height,
                0..INKS.len(),
                proptest::option::weighted(0.85, shelf_draw()),
            );
            let cell = (0..width, 0..height);
            (
                Just(width),
                Just(height),
                proptest::collection::vec(pane, n),
                proptest::collection::vec((0..width, 0..height, 1u16..40, any::<bool>()), 0..10),
                proptest::collection::vec(cell.clone(), 0..3),
                proptest::collection::vec(rect_draw(width, height), 0..3),
                proptest::option::weighted(0.4, rect_draw(width, height)),
                proptest::collection::vec(cell, 0..12),
                proptest::collection::vec(rect_draw(width, height), 0..3),
                proptest::collection::vec((0..KINDS.len(), 0..n, 0u16..=1000), 0..4),
            )
                .prop_map(
                    |(width, height, panes, text, images, protected, chat, layer, made, pieces)| {
                        let panes = panes
                            .into_iter()
                            .map(|(top, bottom, ink, shelf)| {
                                let bottom = bottom.max(top + 3).min(height);
                                (top, bottom.max(top + 3), ink, shelf)
                            })
                            .collect();
                        Draw {
                            width,
                            height,
                            panes,
                            text,
                            images,
                            protected,
                            chat,
                            layer,
                            made,
                            pieces,
                        }
                    },
                )
        })
    }

    fn rect((x, y, w, h): (u16, u16, u16, u16)) -> Rect {
        Rect::new(x, y, w, h)
    }

    /// The scene a draw makes: the frame, the panes, the home with its
    /// shelves, and what's kept off.
    struct Scene {
        buf: Buffer,
        nooks: Vec<(Nook, Rect)>,
        /// Each themed pane's background, as the draw declares it (any
        /// other pane's is the terminal's, `Color::Reset`).
        themes: Vec<(Nook, Color)>,
        home: Home,
        protected: Vec<Rect>,
        chat: Rect,
        layer: Vec<(u16, u16)>,
        made: Vec<Rect>,
    }

    fn scene(draw: &Draw) -> Scene {
        let area = Rect::new(0, 0, draw.width, draw.height);
        let mut buf = Buffer::empty(area);
        let n = u16::try_from(draw.panes.len()).unwrap();
        let mut nooks = Vec::new();
        let mut home = Home::default();
        // What crowds a shelf's margin: wide glyphs `(x, y)`, before the
        // frame's sanitized, and image cells and styled blanks, after.
        let (mut wide, mut images) = (Vec::new(), Vec::new());
        let mut styled: Vec<(Rect, Style)> = Vec::new();
        let mut themes = Vec::new();
        for (i, &(top, bottom, ink, ref shelf)) in draw.panes.iter().enumerate() {
            let i16 = u16::try_from(i).unwrap();
            let x = i16 * draw.width / n;
            let right = (i16 + 1) * draw.width / n;
            let pane = Rect::new(x, top, right - x, bottom.min(draw.height) - top);
            Block::bordered()
                .border_style(Style::new().fg(INKS[ink]))
                .render(pane, &mut buf);
            nooks.push((NOOKS[i], pane));
            let Some(shelf) = shelf else { continue };
            // By construction: an offset and a rise the pane allows, if
            // it allows any (else it's drawn as anything goes), inside or
            // at the edges.
            let inner = i32::from(pane.width) - 2;
            let rows = pane.height.saturating_sub(2);
            let offsets = inner - 2 * EDGE - SHELF_WIDTH + 1;
            let rises = i32::from(rows) - i32::from(LOW_ABOVE) - i32::from(LOW_RISE) + 1;
            let fits = offsets > 0 && rises > 0;
            let (offset, rise) = match shelf.how {
                1 if fits => (
                    u16::try_from(EDGE + i32::from(shelf.offset) % offsets).unwrap(),
                    LOW_RISE + u16::try_from(i32::from(shelf.rise) % rises).unwrap(),
                ),
                2 if fits => {
                    let offsets = [EDGE - 1, EDGE, EDGE + offsets - 1, EDGE + offsets];
                    let top = rows - LOW_ABOVE;
                    let rises = [LOW_RISE - 1, LOW_RISE, top, top + 1];
                    (
                        u16::try_from(offsets[usize::from(shelf.offset % 4)]).unwrap(),
                        rises[usize::from(shelf.rise % 4)],
                    )
                }
                _ => (shelf.offset, shelf.rise),
            };
            let side = if shelf.right { Side::Right } else { Side::Left };
            home.structures.push(Structure::Shelf(Shelf {
                nook: NOOKS[i],
                anchor: Anchor { side, offset },
                rise,
                built: shelf.built,
            }));
            // Its built run's columns, from its record and its pane's rect.
            let from = match side {
                Side::Left => i32::from(pane.x) + 1 + i32::from(offset),
                Side::Right => i32::from(pane.right()) - 1 - i32::from(offset) - 10,
            };
            let (first, last) = match (shelf.built, side) {
                (0, _) => continue,
                (1, Side::Left) => (from, from + 4),
                (1, Side::Right) => (from + 5, from + 9),
                _ => (from, from + 9),
            };
            let y = i32::from(pane.bottom()) - 1 - i32::from(rise);
            match shelf.crowd {
                2 => wide.push((first - 3, y)),
                3 => {
                    let span = last - first + 5;
                    images.push((first - 2 + i32::from(shelf.pick) % span, y));
                }
                4 => styled.push((
                    Rect::new(pane.x + 1, u16::try_from(y).unwrap(), pane.width - 2, 1),
                    Style::new().add_modifier(Modifier::REVERSED),
                )),
                5 => {
                    let span = last - first + 5;
                    let x = first - 2 + i32::from(shelf.pick) % span;
                    styled.push((
                        Rect::new(u16::try_from(x).unwrap(), u16::try_from(y).unwrap(), 1, 1),
                        Style::new().bg(Color::Blue),
                    ));
                }
                6 | 7 => {
                    let theme = Color::Indexed(236);
                    themes.push((NOOKS[i], theme));
                    styled.push((pane, Style::new().bg(theme)));
                    if shelf.crowd == 7 {
                        let span = last - first + 5;
                        let x = first - 2 + i32::from(shelf.pick) % span;
                        styled.push((
                            Rect::new(u16::try_from(x).unwrap(), u16::try_from(y).unwrap(), 1, 1),
                            Style::new().bg(Color::Reset),
                        ));
                    }
                }
                _ => {}
            }
        }
        for &(x, y, len, wide) in &draw.text {
            let text: String = if wide {
                "漢字".repeat(usize::from(len / 2 + 1))
            } else {
                "so what did you think ".repeat(2)
            };
            let len = usize::from(len);
            buf.set_stringn(x, y, &text, len, Style::new());
        }
        let on = |(x, y): (i32, i32)| u16::try_from(x).ok().zip(u16::try_from(y).ok());
        for (x, y) in wide.into_iter().filter_map(on) {
            buf.set_stringn(x, y, "漢", 2, Style::new());
        }
        super::super::cells::sanitize(&mut buf);
        // An image's cells read blank: only their diff option says so.
        let at = draw.images.iter().copied();
        for (x, y) in at.chain(images.into_iter().filter_map(on)) {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ")
                    .set_diff_option(tuirealm::ratatui::buffer::CellDiffOption::Skip);
            }
        }
        for (rect, style) in styled {
            buf.set_style(rect, style);
        }
        for &(kind, pane, at) in &draw.pieces {
            if let Some(&(nook, _)) = nooks.get(pane) {
                home.add(Prop::new(KINDS[kind], nook, at, Facing::Right));
            }
        }
        // Anchored where their shares put them, as a frame's projection
        // pins them before her structures are read.
        home.layout(&nooks);
        Scene {
            buf,
            nooks,
            themes,
            home,
            protected: draw.protected.iter().copied().map(rect).collect(),
            chat: draw.chat.map(rect).unwrap_or_default(),
            layer: draw.layer.clone(),
            made: draw.made.iter().copied().map(rect).collect(),
        }
    }

    /// A cell a shelf writes: where, and its glyph.
    type Cellwise = (i32, i32, char);

    /// What a shelf should write on `scene`'s frame, worked out from its
    /// record and its pane's rect alone: each cell with its glyph, and
    /// its ink; `None` where it can't hang; empty where it hangs but
    /// nothing of it is up.
    fn expected(scene: &Scene, laid: &[Rect], shelf: &Shelf) -> Option<(Vec<Cellwise>, Color)> {
        let pane = scene.nooks.iter().find(|(n, _)| *n == shelf.nook)?.1;
        let (left, right) = (i32::from(pane.x) + 1, i32::from(pane.right()) - 1);
        let floor = i32::from(pane.bottom()) - 1;
        let rows = i32::from(pane.height) - 2;
        let offset = i32::from(shelf.anchor.offset);
        let from = match shelf.anchor.side {
            Side::Left => left + offset,
            Side::Right => right - offset - 10,
        };
        let rise = i32::from(shelf.rise);
        if from < left + 8 || from + 10 > right - 8 || rise < 5 || rows - rise < 2 {
            return None;
        }
        let y = floor - rise;
        let cells: Vec<Cellwise> = match (shelf.built, shelf.anchor.side) {
            (0, _) => Vec::new(),
            (1, Side::Left) => (from..from + 5)
                .map(|x| (x, y, if x == from { '┬' } else { '─' }))
                .collect(),
            (1, Side::Right) => (from + 5..from + 10)
                .map(|x| (x, y, if x == from + 9 { '┬' } else { '─' }))
                .collect(),
            _ => (from..from + 10)
                .map(|x| {
                    (
                        x,
                        y,
                        if x == from || x == from + 9 {
                            '┬'
                        } else {
                            '─'
                        },
                    )
                })
                .collect(),
        };
        let ok = |x: i32| {
            let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(y)) else {
                return false;
            };
            let Some(cell) = scene.buf.cell((ux, uy)) else {
                return false;
            };
            let at = (ux, uy).into();
            let trailer = x > 0
                && scene
                    .buf
                    .cell((ux - 1, uy))
                    .is_some_and(|c| unicode_width::UnicodeWidthStr::width(c.symbol()) > 1);
            // A styled blank (a selection bar, a cell in a colour unlike
            // its pane's) is as good as text: the user's fifth round. Its
            // pane's background as the draw declares it.
            let pane_bg = scene
                .themes
                .iter()
                .find(|(n, _)| *n == shelf.nook)
                .map_or(Color::Reset, |&(_, bg)| bg);
            cell.symbol().trim().is_empty()
                && !cell.modifier.contains(Modifier::REVERSED)
                && cell.bg == pane_bg
                && !trailer
                && !image_cell(cell)
                && !scene.protected.iter().any(|r| r.contains(at))
                && !scene.chat.contains(at)
                && !scene.layer.contains(&(ux, uy))
                && !scene.made.iter().any(|r| r.contains(at))
                && !laid.iter().any(|r| r.contains(at))
        };
        let stands = match (cells.first(), cells.last()) {
            (Some(&(first, ..)), Some(&(last, ..))) => (first - 2..=last + 2).all(ok),
            _ => false,
        };
        let ink = scene
            .buf
            .cell((
                u16::try_from(from + 5).unwrap(),
                u16::try_from(floor).unwrap(),
            ))
            .map_or(Color::Reset, |c| c.fg);
        Some((if stands { cells } else { Vec::new() }, ink))
    }

    /// How often each case came up (non-vacuity).
    #[derive(Debug, Default)]
    struct Seen {
        written: usize,
        half: usize,
        hidden: usize,
    }

    fn takes_only_blank_free_cells(draw: &Draw) -> Result<Seen, TestCaseError> {
        let scene = scene(draw);
        let plan = Plan {
            nooks: &scene.nooks,
            chat: scene.chat,
            screen: scene.buf.area,
        };
        let keep_off = KeepOff {
            protected: &scene.protected,
            layer: &scene.layer,
            made: &scene.made,
        };
        let laid: Vec<Rect> = scene
            .home
            .laid_and_shifted(&scene.nooks)
            .shown
            .iter()
            .map(Shown::cover)
            .collect();
        let mut buf = scene.buf.clone();
        let written = read_ground(&mut buf, &scene.home, plan, &keep_off);
        // Kept at once (a failed assertion below returns early, and
        // what's written mustn't be dropped held).
        let shown_on = written.shown_on().to_vec();
        let mut kept = Vec::new();
        written.keep(&mut kept);
        let mut seen = Seen::default();
        let mut want: Vec<(i32, i32, char, Color)> = Vec::new();
        let mut shown = Vec::new();
        for &Structure::Shelf(shelf) in &scene.home.structures {
            let Some((cells, ink)) = expected(&scene, &laid, &shelf) else {
                continue;
            };
            match (cells.len(), shelf.built) {
                (0, 0) => {}
                (0, _) => seen.hidden += 1,
                (5, 1) => {
                    seen.written += 1;
                    seen.half += 1;
                }
                (10, 2) => seen.written += 1,
                (n, built) => prop_assert!(false, "{n} cells of a shelf {built} built"),
            }
            if !cells.is_empty() {
                shown.push(shelf.nook);
            }
            want.extend(cells.into_iter().map(|(x, y, glyph)| (x, y, glyph, ink)));
        }
        // Exactly those cells changed, to the shelf's glyphs in its
        // pane's border ink; all else as the app drew it.
        for y in 0..scene.buf.area.height {
            for x in 0..scene.buf.area.width {
                let (before, after) = (&scene.buf[(x, y)], &buf[(x, y)]);
                let at = (i32::from(x), i32::from(y));
                let frozen = kept.iter().find(|c| (c.x, c.y) == (x, y));
                match want.iter().find(|w| (w.0, w.1) == at) {
                    Some(&(.., glyph, ink)) => {
                        prop_assert_eq!(after.symbol(), glyph.to_string(), "{:?}", at);
                        prop_assert_eq!(after.fg, ink, "{:?}", at);
                        prop_assert!(after.modifier.is_empty(), "{:?}", at);
                        prop_assert!(before.symbol().trim().is_empty(), "{:?}: over text", at);
                        prop_assert_eq!(frozen.map(|c| &c.under), Some(before), "{:?}", at);
                        prop_assert_eq!(frozen.map(|c| c.glyph), Some(glyph), "{:?}", at);
                    }
                    None => {
                        prop_assert_eq!(after, before, "{:?}: not hers", at);
                        prop_assert!(frozen.is_none(), "{:?}", at);
                    }
                }
            }
        }
        prop_assert_eq!(&shown_on, &shown);
        prop_assert!(kept.iter().all(|c| !c.burst && c.face.is_none()));
        // Kept, they're the bottom layer, before what was painted.
        let mut again = scene.buf.clone();
        let mut painted = vec![Frozen::hole(0, 0, Cell::default())];
        read_ground(&mut again, &scene.home, plan, &keep_off).keep(&mut painted);
        prop_assert_eq!(&painted[..kept.len()], &kept[..]);
        prop_assert_eq!(
            &painted[kept.len()..],
            &[Frozen::hole(0, 0, Cell::default())]
        );
        // Undone, the frame is the app's.
        let mut undone = scene.buf.clone();
        read_ground(&mut undone, &scene.home, plan, &keep_off).undo(&mut undone);
        prop_assert_eq!(&undone, &scene.buf, "undone, the app's frame");
        // With no structures, nothing is read or written.
        let bare = Home {
            structures: Vec::new(),
            ..scene.home.clone()
        };
        let mut untouched = scene.buf.clone();
        let none = read_ground(&mut untouched, &bare, plan, &keep_off);
        prop_assert!(none.shown_on().is_empty());
        none.undo(&mut untouched);
        prop_assert_eq!(&untouched, &scene.buf);
        Ok(seen)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(256)))]

        /// Her shelves are written only on blank cells nothing else
        /// holds (D4's per-cell test), each all of it or none, its run
        /// where its record and its pane put it, its stages as built;
        /// kept, they're the bottom layer; undone, the frame is the
        /// app's; with none, the frame isn't touched.
        #[test]
        fn read_ground_takes_only_blank_free_cells(draw in draw()) {
            takes_only_blank_free_cells(&draw)?;
        }
    }

    /// A shelf hung in `nooks`' Users pane (`Left` 12, rise 7: run
    /// `63..73` on row 5 of [`shelf_frame`]), `built` stages up.
    fn users_shelf(built: u8) -> Home {
        Home {
            structures: vec![Structure::Shelf(Shelf {
                nook: Nook::Users,
                anchor: Anchor {
                    side: Side::Left,
                    offset: 12,
                },
                rise: 7,
                built,
            })],
            ..Home::default()
        }
    }

    /// A 100×30 frame with a Users pane (`50, 0, 50, 13`), text in it,
    /// and a text floor (`─`) on its row 9, where her box standing would
    /// meet a shelf on row 5.
    fn shelf_frame() -> (Buffer, [(Nook, Rect); 1]) {
        let mut real = Buffer::empty(Rect::new(0, 0, 100, 30));
        let pane = Rect::new(50, 0, 50, 13);
        Block::bordered().render(pane, &mut real);
        real.set_string(52, 2, "so what did you think", Style::new());
        real.set_string(52, 9, "─".repeat(28), Style::new());
        real.set_string(80, 10, "it was fine", Style::new());
        (real, [(Nook::Users, pane)])
    }

    /// A shelf is no floor of hers in 6a (D5): with ≥ 4 rows over it,
    /// where she'd fit standing, the frame with it written reads as the
    /// frame without it, platforms and links alike; its cells are lines
    /// (her image redraws them) and her structures', but neither ledge
    /// nor pole (its brackets give no climb). Both modes.
    #[test]
    fn a_shelf_is_no_floor_of_hers() {
        use super::super::door::Ground;
        use super::super::terrain::Terrain;
        for graphics in [false, true] {
            let (real, nooks) = shelf_frame();
            let plan = Plan {
                nooks: &nooks,
                chat: Rect::default(),
                screen: real.area,
            };
            let keep_off = KeepOff {
                protected: &[],
                layer: &[],
                made: &[],
            };
            let mut buf = real.clone();
            let written = read_ground(&mut buf, &users_shelf(2), plan, &keep_off);
            let run: String = (63..73).map(|x| buf[(x, 5)].symbol().to_owned()).collect();
            assert_eq!(run, "┬────────┬", "graphics={graphics}: written");
            let with = Terrain::read_with(&buf, &[], graphics, &written);
            let without = Terrain::read(&real, &[], graphics);
            assert_eq!(with.platforms, without.platforms, "graphics={graphics}");
            assert_eq!(with.links, without.links, "graphics={graphics}");
            for x in 63..73 {
                assert!(!with.ledge(x, 5), "graphics={graphics}: x {x} a ledge");
                assert!(
                    with.structure(x, 5) && with.stroke(x, 5),
                    "graphics={graphics}: x {x}"
                );
                assert!(!without.structure(x, 5), "graphics={graphics}: x {x}");
            }
            assert!(
                !with.links.iter().any(|l| l.pole == 63 || l.pole == 72),
                "graphics={graphics}: a bracket's a pole"
            );
            // Under it, on the text floor, she still stands.
            assert!(
                with.platforms.iter().any(|p| p.y == 9 && p.contains(68)),
                "graphics={graphics}: under it"
            );
            written.undo(&mut buf);
            assert_eq!(buf, real, "graphics={graphics}");
        }
    }

    /// A bracket is no pole of hers (D5; the user's note: its stubs never
    /// count as climbable): a floor of text over a bracket's column and
    /// another under it, the column a text `│` between them but for the
    /// shelf's row, give no climb with the shelf written there, though
    /// the same column with a `│` of text on that row does (the fixture
    /// would link: not vacuous). Each bracket of each stage, both modes.
    #[test]
    fn a_bracket_is_no_pole_of_hers() {
        use super::super::terrain::{Route, Terrain};
        // Users `50, 0, 50, 22`: floor 21, so rise 7 hangs the run
        // `63..73` on row 14.
        let pane = Rect::new(50, 0, 50, 22);
        let nooks = [(Nook::Users, pane)];
        let plan = Plan {
            nooks: &nooks,
            chat: Rect::default(),
            screen: Rect::new(0, 0, 100, 30),
        };
        let keep_off = KeepOff {
            protected: &[],
            layer: &[],
            made: &[],
        };
        let climbs = |terrain: &Terrain, column: i32| {
            terrain
                .links
                .iter()
                .any(|l| l.route == Route::Climb && l.pole == column)
        };
        for graphics in [false, true] {
            for (built, column) in [(1, 63), (2, 63), (2, 72)] {
                let at = format!("graphics={graphics} built={built} column={column}");
                let mut real = Buffer::empty(plan.screen);
                Block::bordered().render(pane, &mut real);
                let x = u16::try_from(column).unwrap();
                real.set_string(56, 9, "─".repeat(21), Style::new());
                real.set_string(56, 19, "─".repeat(21), Style::new());
                for y in (10..19).filter(|&y| y != 14) {
                    real.set_string(x, y, "│", Style::new());
                }
                // With a `│` of text on the shelf's row: a climb.
                let mut pole = real.clone();
                pole.set_string(x, 14, "│", Style::new());
                let linked = Terrain::read(&pole, &[], graphics);
                assert!(climbs(&linked, column), "{at}: the fixture links");
                // With the shelf written there: none.
                let mut buf = real.clone();
                let written = read_ground(&mut buf, &users_shelf(built), plan, &keep_off);
                assert_eq!(buf[(x, 14)].symbol(), "┬", "{at}: the bracket written");
                let with = Terrain::read_with(&buf, &[], graphics, &written);
                assert!(!climbs(&with, column), "{at}: a bracket's a pole");
                assert_eq!(
                    with.links,
                    Terrain::read(&real, &[], graphics).links,
                    "{at}: as if it weren't there"
                );
                written.undo(&mut buf);
            }
        }
    }

    /// One pane `0, 0, 60, 20` (18 rows inside, 58 columns) with `shelf`
    /// and nothing else on the frame: a shelf's geometry holds there for
    /// an offset of 8 to 40 and a rise of 5 to 16.
    fn alone(shelf: ShelfDraw) -> Draw {
        Draw {
            width: 60,
            height: 24,
            panes: vec![(0, 20, 1, Some(shelf))],
            text: Vec::new(),
            images: Vec::new(),
            protected: Vec::new(),
            chat: None,
            layer: Vec::new(),
            made: Vec::new(),
            pieces: Vec::new(),
        }
    }

    fn as_drawn(right: bool, offset: u16, rise: u16, built: u8, crowd: u8, pick: u16) -> ShelfDraw {
        ShelfDraw {
            how: 0,
            right,
            offset,
            rise,
            built,
            crowd,
            pick,
        }
    }

    /// A shelf's geometry at its edges, either side of each, against the
    /// oracle: it hangs exactly where its run and margin keep `EDGE` from
    /// both walls and `LOW_ABOVE` rows are over it, at a rise of
    /// `LOW_RISE` or more. Both sides, both stages.
    #[test]
    fn a_shelf_hangs_exactly_where_its_geometry_holds() {
        for right in [false, true] {
            for offset in [7, 8, 40, 41] {
                for rise in [4, 5, 16, 17] {
                    for built in [1, 2] {
                        let draw = alone(as_drawn(right, offset, rise, built, 0, 0));
                        let at = format!("right={right} offset={offset} rise={rise} built={built}");
                        let seen = takes_only_blank_free_cells(&draw).expect(&at);
                        let holds = (8..=40).contains(&offset) && (5..=16).contains(&rise);
                        assert_eq!(seen.written, usize::from(holds), "{at}");
                    }
                }
            }
        }
    }

    /// What reads blank but isn't free hides a shelf: an image's cell on
    /// any cell of its built run or margin, and the second half of a wide
    /// glyph on the first cell of its margin (the glyph itself outside
    /// it). Both sides, both stages, against the oracle.
    #[test]
    fn a_shelf_keeps_off_images_and_wide_glyphs() {
        for right in [false, true] {
            for built in [1, 2] {
                let at = format!("right={right} built={built}");
                let clear =
                    takes_only_blank_free_cells(&alone(as_drawn(right, 20, 7, built, 0, 0)));
                assert_eq!(clear.expect(&at).written, 1, "{at}: with nothing there");
                let trailer = alone(as_drawn(right, 20, 7, built, 2, 0));
                let seen = takes_only_blank_free_cells(&trailer).expect(&at);
                assert_eq!(
                    (seen.written, seen.hidden),
                    (0, 1),
                    "{at}: a wide glyph's half"
                );
                let span = if built == 1 { 9 } else { 14 };
                for pick in 0..span {
                    let image = alone(as_drawn(right, 20, 7, built, 3, pick));
                    let seen = takes_only_blank_free_cells(&image).expect(&at);
                    assert_eq!(
                        (seen.written, seen.hidden),
                        (0, 1),
                        "{at}: an image at {pick}"
                    );
                }
            }
        }
    }

    /// A styled blank hides a shelf as text does (the user's fifth round):
    /// a selection bar in reverse video across its row (a list's
    /// highlighted entry, past its text), or a blank in a colour unlike
    /// its pane's on any cell of its built run or margin. A pane in a
    /// theme's background of its own (a layout's `background-color`)
    /// holds its shelf on blanks in that background, and a blank in the
    /// terminal's there is one unlike its pane's. Both sides, both
    /// stages, against the oracle (which takes the pane's background
    /// from the draw, not the frame).
    #[test]
    fn a_styled_blank_hides_a_shelf() {
        for right in [false, true] {
            for built in [1, 2] {
                let at = format!("right={right} built={built}");
                let bar = alone(as_drawn(right, 20, 7, built, 4, 0));
                let seen = takes_only_blank_free_cells(&bar).expect(&at);
                assert_eq!((seen.written, seen.hidden), (0, 1), "{at}: a selection bar");
                let themed = alone(as_drawn(right, 20, 7, built, 6, 0));
                let seen = takes_only_blank_free_cells(&themed).expect(&at);
                assert_eq!((seen.written, seen.hidden), (1, 0), "{at}: a themed pane");
                let span = if built == 1 { 9 } else { 14 };
                for pick in 0..span {
                    for (crowd, what) in [(5, "a tinted blank"), (7, "a terminal's blank")] {
                        let tinted = alone(as_drawn(right, 20, 7, built, crowd, pick));
                        let seen = takes_only_blank_free_cells(&tinted).expect(&at);
                        assert_eq!(
                            (seen.written, seen.hidden),
                            (0, 1),
                            "{at}: {what} at {pick}"
                        );
                    }
                }
            }
        }
    }

    /// The property's non-vacuity, over a fixed run of its strategy:
    /// shelves written whole and half-built, and shelves whose geometry
    /// holds hidden by what's on the frame (26, 15 and 38 of them as the
    /// strategy stands; each threshold about half that).
    #[test]
    fn read_ground_writes_and_hides_shelves() {
        let mut runner = TestRunner::new_with_rng(
            Config::default(),
            TestRng::deterministic_rng(proptest::test_runner::RngAlgorithm::ChaCha),
        );
        let mut seen = Seen::default();
        for _ in 0..256 {
            let draw = draw().new_tree(&mut runner).unwrap().current();
            let once = takes_only_blank_free_cells(&draw).unwrap();
            seen.written += once.written;
            seen.half += once.half;
            seen.hidden += once.hidden;
        }
        assert!(
            seen.written >= 13 && seen.half >= 7 && seen.hidden >= 19,
            "{seen:?}"
        );
    }
}
