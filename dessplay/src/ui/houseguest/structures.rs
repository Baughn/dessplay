//! Her structures (phase 6): what she puts up in her home with the UI's
//! own line glyphs. In 6a, one shelf on a pane's back wall
//! (`Structure::Shelf`), saved in her record; the frame's cells it's
//! drawn in are what `Written` holds (phase 6a D4: written once a paint,
//! before anything reads the frame; this build writes none yet, so every
//! read is of the real frame).

use serde::{Deserialize, Serialize};
use tuirealm::ratatui::buffer::{Buffer, Cell};

use super::dissolve::Frozen;
use super::room::{Anchor, Nook};

/// A shelf's run: `┬────────┬`, its brackets at both ends.
#[cfg(test)] // Read from step 2b (its geometry, D3).
pub(super) const SHELF_WIDTH: i32 = 10;

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
#[cfg(test)] // Read from step 2b (its geometry, D3).
pub(super) const LOW_ABOVE: u16 = 2;

/// How far in from either wall of its pane a shelf's run starts, at
/// least: a body or a door's space (`max(WIDTH, SPACE)`), and the 2
/// cells of its margin, so no door's space ever meets it.
#[cfg(test)] // Read from step 2b (its geometry, D3).
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
}

/// What her structures wrote into the frame this paint: each cell, with
/// the real cell it covers. This build writes none (`none`): it's
/// threaded to every terrain read and every `under` a cell of hers
/// records, for step 2b's write (the layout's text checks take it from
/// step 3, D6).
#[must_use = "an arm keeps it (into its painted list) or undoes it"]
#[derive(Debug, Default)]
pub(super) struct Written {
    cells: Vec<Frozen>,
}

impl Written {
    /// Nothing written: the frame is the app's own.
    pub fn none() -> Self {
        Self::default()
    }

    /// What the app drew at `(x, y)`, under anything written there: what
    /// a cell of hers drawn over it records as its `under`.
    pub fn real(&self, buf: &Buffer, x: u16, y: u16) -> Option<Cell> {
        match self.cells.iter().find(|c| (c.x, c.y) == (x, y)) {
            Some(written) => Some(written.under.clone()),
            None => buf.cell((x, y)).cloned(),
        }
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
