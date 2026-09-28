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

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(128)))]

        /// Whatever she tries: no two entries share a cell, every target
        /// was free in the real frame, and painting only changes her
        /// entries' cells.
        #[test]
        fn the_layer_never_overlaps_or_covers_text(
            rows in proptest::collection::vec("[a-z漢 ]{12}", 4),
            moves in proptest::collection::vec(((0u16..12, 0u16..4), (0u16..12, 0u16..4)), 0..40),
            guard in (0u16..12, 0u16..4, 1u16..4, 1u16..3),
        ) {
            let lines: Vec<&str> = rows.iter().map(String::as_str).collect();
            let real = text(&lines);
            let protected = [Rect::new(guard.0, guard.1, guard.2, guard.3).intersection(real.area)];
            let mut layer = TextLayer::default();
            for (from, to) in moves {
                if !layer.shift(&real, &protected, from, to) {
                    layer.take(&real, &protected, from, to);
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
