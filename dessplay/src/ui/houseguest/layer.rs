//! The text layer: glyphs she has moved (and the holes they left),
//! composed over each frame and validated against it first.
//!
//! Each entry owns exactly two places: the **source** cell it came from
//! (remembering what the real UI showed there) and the cell it sits
//! **at** now. Every frame, before anything is written, an entry is
//! dropped if its source no longer shows what it took, or if its target
//! is no longer free — so she can never paint a stale hole over changed
//! text, nor a glyph over new text. Wide glyphs move whole, as two-cell
//! bricks.

use tuirealm::ratatui::buffer::{Buffer, Cell};
use tuirealm::ratatui::layout::{Position, Rect};
use tuirealm::ratatui::style::Style;

use super::cells::{untouchable, width};
use super::dissolve::Frozen;

/// Most glyphs she moves at once.
pub(super) const CAP: usize = 60;

/// One moved glyph.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Displaced {
    /// Where it came from, and the real cell there when she took it.
    pub source: (u16, u16),
    pub expected: Cell,
    /// Where it sits now.
    pub at: (u16, u16),
}

impl Displaced {
    fn wide(&self) -> bool {
        width(&self.expected) > 1
    }

    /// The cells it occupies at `at` (two for a wide glyph).
    fn at_cells(&self) -> impl Iterator<Item = (u16, u16)> + '_ {
        let (x, y) = self.at;
        std::iter::once((x, y)).chain(self.wide().then(|| (x.saturating_add(1), y)))
    }

    fn source_cells(&self) -> impl Iterator<Item = (u16, u16)> + '_ {
        let (x, y) = self.source;
        std::iter::once((x, y)).chain(self.wide().then(|| (x.saturating_add(1), y)))
    }
}

/// A glyph as shown: the real cell it came from (`source`), and where it sits
/// (`at == source` when it's home).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Placed {
    pub source: (u16, u16),
    pub at: (u16, u16),
}

impl Placed {
    /// A glyph at home.
    pub fn home(source: (u16, u16)) -> Self {
        Self { source, at: source }
    }
}

/// Whether the real cell at `(x, y)` is free for a moved glyph: on
/// screen, blank, touchable, and not protected.
pub(super) fn free(buf: &Buffer, protected: &[Rect], (x, y): (u16, u16)) -> bool {
    let position = Position::new(x, y);
    let blank_trailing_ok = x
        .checked_sub(1)
        .and_then(|left| buf.cell((left, y)))
        .is_none_or(|left| width(left) <= 1);
    buf.cell(position)
        .is_some_and(|cell| cell.symbol().trim().is_empty() && !untouchable(cell))
        && blank_trailing_ok
        && !protected.iter().any(|r| r.contains(position))
}

/// Whether the real cell at `(x, y)` holds a glyph she may pick up: one
/// visible character (combining marks and image cells refused), the
/// leading half of a wide glyph rather than its trailing blank, and not
/// protected.
pub(super) fn takeable(buf: &Buffer, protected: &[Rect], (x, y): (u16, u16)) -> bool {
    let position = Position::new(x, y);
    let Some(cell) = buf.cell(position) else {
        return false;
    };
    let mut chars = cell.symbol().chars();
    let single = matches!((chars.next(), chars.next()), (Some(c), None) if !c.is_whitespace());
    single && !untouchable(cell) && !protected.iter().any(|r| r.contains(position))
}

/// Her moved glyphs.
#[derive(Clone, Debug, Default)]
pub(super) struct TextLayer {
    entries: Vec<Displaced>,
}

impl TextLayer {
    /// The entries.
    #[cfg(test)]
    pub fn entries(&self) -> &[Displaced] {
        &self.entries
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Where an entry that came from `source` sits now.
    pub fn at_of(&self, source: (u16, u16)) -> Option<(u16, u16)> {
        self.entries
            .iter()
            .find(|d| d.source == source)
            .map(|d| d.at)
    }

    /// Where the glyph shown at `at` came from, if it's one she moved.
    pub fn shown_from(&self, at: (u16, u16)) -> Option<(u16, u16)> {
        self.entries.iter().find(|d| d.at == at).map(|d| d.source)
    }

    /// The holes she left that nothing of hers covers.
    pub fn holes(&self) -> impl Iterator<Item = (u16, u16)> + '_ {
        let covered: Vec<(u16, u16)> = self.entries.iter().flat_map(|d| d.at_cells()).collect();
        self.entries
            .iter()
            .flat_map(|d| d.source_cells().collect::<Vec<_>>())
            .filter(move |c| !covered.contains(c))
    }

    /// Every cell the layer uses: sources (holes) and targets.
    pub fn cells(&self) -> impl Iterator<Item = (u16, u16)> + '_ {
        self.entries
            .iter()
            .flat_map(|d| d.at_cells().chain(d.source_cells()).collect::<Vec<_>>())
    }

    /// Whether `entry` may sit where it says, given the other entries:
    /// each target cell is unused by other targets, and is either free in
    /// the real frame or a hole some entry (itself included) left.
    fn fits(
        &self,
        buf: &Buffer,
        protected: &[Rect],
        entry: &Displaced,
        except: Option<usize>,
    ) -> bool {
        let others = self
            .entries
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != except)
            .map(|(_, d)| d);
        let mut targets = Vec::new();
        let mut holes: Vec<(u16, u16)> = entry.source_cells().collect();
        for d in others {
            targets.extend(d.at_cells());
            holes.extend(d.source_cells());
        }
        entry.at_cells().all(|c| {
            let position = Position::new(c.0, c.1);
            !targets.contains(&c)
                && !protected.iter().any(|r| r.contains(position))
                && (holes.contains(&c) || free(buf, protected, c))
        })
    }

    /// Pick up the glyph at `source` and put it at `to`. Refused unless
    /// the glyph is takeable, it actually moves, its target fits, and the
    /// cap allows.
    pub fn take(
        &mut self,
        buf: &Buffer,
        protected: &[Rect],
        source: (u16, u16),
        to: (u16, u16),
    ) -> bool {
        if self.entries.len() >= CAP
            || to == source
            || self
                .entries
                .iter()
                .any(|d| d.source_cells().any(|c| c == source))
            || !takeable(buf, protected, source)
        {
            return false;
        }
        let Some(expected) = buf.cell(source).cloned() else {
            return false;
        };
        let entry = Displaced {
            source,
            expected,
            at: to,
        };
        let ok = self.fits(buf, protected, &entry, None);
        if ok {
            self.entries.push(entry);
        }
        ok
    }

    /// Move the entry from `source` to `to` (checked like [`Self::take`]).
    /// Moving it back onto its own source returns it home.
    pub fn shift(
        &mut self,
        buf: &Buffer,
        protected: &[Rect],
        source: (u16, u16),
        to: (u16, u16),
    ) -> bool {
        let Some(index) = self.entries.iter().position(|d| d.source == source) else {
            return false;
        };
        if to == source {
            return self.restore(source);
        }
        let Some(entry) = self.entries.get(index) else {
            return false;
        };
        let moved = Displaced {
            at: to,
            ..entry.clone()
        };
        let ok = self.fits(buf, protected, &moved, Some(index));
        if ok && let Some(slot) = self.entries.get_mut(index) {
            *slot = moved;
        }
        ok
    }

    /// Put the glyph from `source` back where it came from — refused
    /// while another glyph sits in its hole.
    pub fn restore(&mut self, source: (u16, u16)) -> bool {
        let Some(entry) = self.entries.iter().find(|d| d.source == source) else {
            return false;
        };
        let home: Vec<_> = entry.source_cells().collect();
        let blocked = self
            .entries
            .iter()
            .any(|d| d.source != source && d.at_cells().any(|c| home.contains(&c)));
        if !blocked {
            self.entries.retain(|d| d.source != source);
        }
        !blocked
    }

    /// Trade the glyphs shown as `a` and `b`: each moves to the other's
    /// place. A glyph at home must be one she may take and hasn't; one
    /// she moved must still be where `a`/`b` says. Both go or neither
    /// does.
    pub fn swap(&mut self, buf: &Buffer, protected: &[Rect], a: Placed, b: Placed) -> bool {
        let before = self.entries.clone();
        let mut fresh = 0;
        for p in [a, b] {
            let ok = if p.at == p.source {
                let used = self
                    .entries
                    .iter()
                    .any(|d| d.source_cells().chain(d.at_cells()).any(|c| c == p.source));
                let taken = (!used && takeable(buf, protected, p.source))
                    .then(|| buf.cell(p.source).cloned())
                    .flatten()
                    .filter(|cell| width(cell) <= 1);
                taken.map(|expected| {
                    fresh += 1;
                    self.entries.push(Displaced {
                        source: p.source,
                        expected,
                        at: p.at,
                    });
                })
            } else {
                self.entries
                    .iter()
                    .find(|d| d.source == p.source && d.at == p.at && !d.wide())
                    .map(|_| ())
            };
            if ok.is_none() || a.source == b.source || before.len() + fresh > CAP {
                self.entries = before;
                return false;
            }
        }
        let traded = [
            Placed {
                source: a.source,
                at: b.at,
            },
            Placed {
                source: b.source,
                at: a.at,
            },
        ];
        if self.place(buf, protected, &traded) {
            return true;
        }
        self.entries = before;
        false
    }

    /// Put each glyph in `to` where it says (home when `at == source`),
    /// all at once — refused, changing nothing, unless every one then
    /// fits and no other glyph sits in a hole that's closing. A source
    /// no longer in the layer is skipped: it's home already, and this
    /// never picks anything up.
    pub fn place(&mut self, buf: &Buffer, protected: &[Rect], to: &[Placed]) -> bool {
        let before = self.entries.clone();
        let mut moved = Vec::new();
        for p in to {
            if let Some(d) = self.entries.iter_mut().find(|d| d.source == p.source) {
                d.at = p.at;
                moved.push(p.source);
            }
        }
        let (home, away): (Vec<Displaced>, Vec<Displaced>) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|d| d.at == d.source);
        self.entries = away;
        let closing: Vec<(u16, u16)> = home
            .iter()
            .flat_map(|d| d.source_cells().collect::<Vec<_>>())
            .collect();
        let blocked = self
            .entries
            .iter()
            .any(|d| d.at_cells().any(|c| closing.contains(&c)));
        let fits = (0..self.entries.len()).all(|i| {
            self.entries.get(i).is_some_and(|d| {
                !moved.contains(&d.source) || self.fits(buf, protected, d, Some(i))
            })
        });
        if blocked || !fits {
            self.entries = before;
            return false;
        }
        true
    }

    /// Drop every entry the real frame no longer supports — a source that
    /// changed, or a target that stopped fitting — repeating until stable,
    /// since a dropped entry's hole fills with real text again. Reads only
    /// the real frame; call before anything of hers is painted.
    pub fn validate(&mut self, buf: &Buffer, protected: &[Rect]) -> usize {
        let before = self.entries.len();
        loop {
            let count = self.entries.len();
            let sources_ok: Vec<bool> = self
                .entries
                .iter()
                .map(|d| {
                    buf.cell(d.source) == Some(&d.expected)
                        && !protected
                            .iter()
                            .any(|r| r.contains(Position::new(d.source.0, d.source.1)))
                })
                .collect();
            let mut index = 0;
            self.entries.retain(|_| {
                let keep = sources_ok.get(index).copied().unwrap_or(false);
                index += 1;
                keep
            });
            let fits: Vec<bool> = (0..self.entries.len())
                .map(|i| {
                    self.entries
                        .get(i)
                        .is_some_and(|d| self.fits(buf, protected, d, Some(i)))
                })
                .collect();
            let mut index = 0;
            self.entries.retain(|_| {
                let keep = fits.get(index).copied().unwrap_or(false);
                index += 1;
                keep
            });
            if self.entries.len() == count {
                break;
            }
        }
        before - self.entries.len()
    }

    /// Paint the layer over the (validated) real frame, returning the
    /// cells changed with what was beneath — the goodbye's frozen cells,
    /// so the rain re-knits the real text.
    pub fn paint(&self, buf: &mut Buffer) -> Vec<Frozen> {
        let mut frozen = Vec::new();
        let mut unders = Vec::new();
        for d in &self.entries {
            for c in d.source_cells().chain(d.at_cells()) {
                if let Some(cell) = buf.cell(c) {
                    unders.push((c, cell.clone()));
                }
            }
        }
        let under = |c: (u16, u16)| {
            unders
                .iter()
                .find(|(p, _)| *p == c)
                .map(|(_, cell)| cell.clone())
        };
        // Holes first, so a glyph moved within its own run lands on top.
        for d in &self.entries {
            for (x, y) in d.source_cells() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ");
                }
                if let Some(real) = under((x, y)) {
                    frozen.push(Frozen::hole(x, y, real));
                }
            }
        }
        for d in &self.entries {
            let (x, y) = d.at;
            let style: Style = d.expected.style();
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(d.expected.symbol()).set_style(style);
            }
            if d.wide()
                && let Some(cell) = buf.cell_mut((x.saturating_add(1), y))
            {
                cell.set_symbol(" ");
            }
            if let Some(real) = under((x, y)) {
                frozen.retain(|f| (f.x, f.y) != (x, y));
                frozen.push(Frozen::glyph(x, y, &d.expected, real));
            }
        }
        frozen
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn text(rows: &[&str]) -> Buffer {
        let mut buf = Buffer::with_lines(rows.iter().copied());
        super::super::cells::sanitize(&mut buf);
        buf
    }

    #[test]
    fn a_moved_glyph_leaves_a_hole_and_paints_at_its_target() {
        let real = text(&["ab   "]);
        let mut layer = TextLayer::default();
        assert!(layer.take(&real, &[], (1, 0), (4, 0)));
        let mut frame = real.clone();
        layer.validate(&frame, &[]);
        let frozen = layer.paint(&mut frame);
        assert_eq!(frame, text(&["a   b"]));
        assert_eq!(frozen.len(), 2);
    }

    #[test]
    fn a_changed_source_drops_the_entry() {
        let real = text(&["ab   "]);
        let mut layer = TextLayer::default();
        assert!(layer.take(&real, &[], (1, 0), (4, 0)));
        let changed = text(&["aX   "]);
        assert_eq!(layer.validate(&changed, &[]), 1);
        assert!(layer.is_empty());
    }

    #[test]
    fn a_target_that_filled_up_drops_the_entry() {
        let real = text(&["ab   "]);
        let mut layer = TextLayer::default();
        assert!(layer.take(&real, &[], (1, 0), (4, 0)));
        assert_eq!(layer.validate(&text(&["ab  !"]), &[]), 1);
    }

    #[test]
    fn wide_glyphs_move_whole_and_need_two_free_cells() {
        let real = text(&["漢   "]);
        let mut layer = TextLayer::default();
        assert!(
            !layer.take(&real, &[], (1, 0), (3, 0)),
            "a trailing half is not a glyph"
        );
        assert!(
            !layer.take(&real, &[], (0, 0), (4, 0)),
            "no room for the second cell"
        );
        assert!(layer.take(&real, &[], (0, 0), (3, 0)));
        let mut frame = real.clone();
        layer.paint(&mut frame);
        assert_eq!(frame, text(&["   漢"]));
    }

    /// Pulling a line one cell right: each glyph moves into the hole its
    /// neighbour left. If the rightmost glyph's source changes, the ones
    /// sitting in holes behind it go too.
    #[test]
    fn a_line_moves_through_its_own_holes() {
        let real = text(&["abc   "]);
        let mut layer = TextLayer::default();
        for x in (0..3u16).rev() {
            assert!(layer.take(&real, &[], (x, 0), (x + 1, 0)), "glyph {x}");
        }
        let mut frame = real.clone();
        layer.paint(&mut frame);
        assert_eq!(frame, text(&[" abc  "]));
        assert!(!layer.restore((2, 0)), "b sits in c's hole");
        layer.validate(&text(&["abX   "]), &[]);
        assert!(layer.is_empty(), "the chain unravels");
    }

    #[test]
    fn protected_cells_are_neither_taken_nor_targeted() {
        let real = text(&["ab   "]);
        let guard = [Rect::new(3, 0, 2, 1)];
        let mut layer = TextLayer::default();
        assert!(!layer.take(&real, &guard, (1, 0), (4, 0)));
        assert!(!layer.take(&real, &[Rect::new(1, 0, 1, 1)], (1, 0), (2, 0)));
    }

    /// The glyph shown at `at`, as a swap names it.
    fn shown(layer: &TextLayer, at: (u16, u16)) -> Placed {
        Placed {
            source: layer.shown_from(at).unwrap_or(at),
            at,
        }
    }

    fn composed(layer: &mut TextLayer, real: &Buffer) -> Buffer {
        let mut frame = real.clone();
        layer.validate(&frame, &[]);
        layer.paint(&mut frame);
        frame
    }

    #[test]
    fn a_swap_trades_two_letters_and_restores_as_a_pair() {
        let real = text(&["the  "]);
        let mut layer = TextLayer::default();
        let (a, b) = (Placed::home((1, 0)), Placed::home((2, 0)));
        assert!(layer.swap(&real, &[], a, b));
        assert_eq!(composed(&mut layer, &real), text(&["teh  "]));
        assert!(!layer.restore((1, 0)), "each sits in the other's hole");
        assert!(layer.place(&real, &[], &[a, b]));
        assert!(layer.is_empty());
    }

    /// Letters she moved (a pulled line) trade where they sit, and the
    /// swap's undo puts them back there, not home.
    #[test]
    fn a_swap_of_moved_letters_trades_them_where_they_sit() {
        let real = text(&["cat    "]);
        let mut layer = TextLayer::default();
        for c in (0..3).rev() {
            assert!(layer.take(&real, &[], (c, 0), (c + 3, 0)));
        }
        assert_eq!(composed(&mut layer, &real), text(&["   cat "]));
        let (a, b) = (shown(&layer, (4, 0)), shown(&layer, (5, 0)));
        assert_eq!(a.source, (1, 0));
        assert!(layer.swap(&real, &[], a, b));
        assert_eq!(composed(&mut layer, &real), text(&["   cta "]));
        assert_eq!(layer.entries().len(), 3, "no new entries");
        assert!(!layer.swap(&real, &[], a, b), "not where they were");
        assert!(layer.place(&real, &[], &[a, b]));
        assert_eq!(composed(&mut layer, &real), text(&["   cat "]));
    }

    /// A moved letter and one at home trade too.
    #[test]
    fn a_moved_letter_trades_with_one_at_home() {
        let real = text(&["ab x "]);
        let mut layer = TextLayer::default();
        assert!(layer.take(&real, &[], (1, 0), (2, 0)));
        let (a, b) = (shown(&layer, (2, 0)), shown(&layer, (3, 0)));
        assert!(layer.swap(&real, &[], a, b));
        assert_eq!(composed(&mut layer, &real), text(&["a xb "]));
        assert!(layer.place(&real, &[], &[a, b]));
        assert_eq!(composed(&mut layer, &real), text(&["a bx "]));
        assert_eq!(layer.entries().len(), 1);
    }

    #[test]
    fn a_swap_is_all_or_nothing() {
        let real = text(&["t漢e  "]);
        let mut layer = TextLayer::default();
        assert!(
            !layer.swap(&real, &[], Placed::home((0, 0)), Placed::home((1, 0))),
            "wide glyphs don't swap"
        );
        assert!(
            !layer.swap(&real, &[], Placed::home((0, 0)), Placed::home((4, 0))),
            "a blank isn't a letter"
        );
        assert!(
            !layer.swap(
                &real,
                &[Rect::new(3, 0, 1, 1)],
                Placed::home((0, 0)),
                Placed::home((3, 0))
            ),
            "protected"
        );
        assert!(layer.is_empty());
        let (a, b) = (Placed::home((0, 0)), Placed::home((3, 0)));
        assert!(layer.swap(&real, &[], a, b));
        assert!(!layer.swap(&real, &[], b, a), "already swapped");
        assert_eq!(layer.entries().len(), 2);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(128)))]

        /// Whatever she tries: no two entries share a cell, every target
        /// was free in the real frame, and painting only changes her
        /// entries' cells.
        #[test]
        fn the_layer_never_overlaps_or_covers_text(
            rows in proptest::collection::vec("[a-z漢 ]{12}", 4),
            moves in proptest::collection::vec((0u8..4, (0u16..12, 0u16..4), (0u16..12, 0u16..4)), 0..40),
            guard in (0u16..12, 0u16..4, 1u16..4, 1u16..3),
        ) {
            let lines: Vec<&str> = rows.iter().map(String::as_str).collect();
            let real = text(&lines);
            let protected = [Rect::new(guard.0, guard.1, guard.2, guard.3).intersection(real.area)];
            let mut layer = TextLayer::default();
            for (kind, from, to) in moves {
                match kind {
                    0 => {
                        let (a, b) = (shown(&layer, from), shown(&layer, to));
                        layer.swap(&real, &protected, a, b);
                    }
                    1 => {
                        // Putting back never picks anything up, whatever
                        // it's told (an undo after the text changed).
                        let before = layer.entries().len();
                        let to = [Placed { source: from, at: to }, Placed::home(to)];
                        layer.place(&real, &protected, &to);
                        prop_assert!(layer.entries().len() <= before);
                    }
                    _ => {
                        if !layer.shift(&real, &protected, from, to) {
                            layer.take(&real, &protected, from, to);
                        }
                    }
                }
            }
            layer.validate(&real, &protected);
            let mut targets = std::collections::HashSet::new();
            let mut sources = std::collections::HashSet::new();
            for d in layer.entries() {
                for c in d.source_cells() {
                    prop_assert!(sources.insert(c), "source {:?} used twice", c);
                }
            }
            for d in layer.entries() {
                for c in d.at_cells() {
                    prop_assert!(targets.insert(c), "target {:?} used twice", c);
                    prop_assert!(!protected[0].contains(Position::new(c.0, c.1)));
                    prop_assert!(sources.contains(&c) || free(&real, &protected, c), "target {:?} covers text", c);
                }
            }
            let seen: std::collections::HashSet<_> = targets.union(&sources).copied().collect();
            let mut frame = real.clone();
            layer.paint(&mut frame);
            for y in 0..4u16 {
                for x in 0..12u16 {
                    if frame.cell((x, y)) != real.cell((x, y)) {
                        prop_assert!(seen.contains(&(x, y)), "({}, {}) changed outside her entries", x, y);
                    }
                }
            }
        }
    }
}
