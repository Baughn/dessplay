//! Drag-to-reorder: the one press/drag/release state machine behind
//! every mouse-reorderable list (the playlist, the settings media roots).
//!
//! A drag is a *preview*: while the button is held the owner renders its
//! rows through [`DragReorder::preview`], and on release it commits the
//! single identity-anchored [`Move`] that [`DragReorder::release`]
//! returns — one synced mutation per gesture, never one per row crossed.
//! Two properties are structural rather than tested-for:
//!
//! - The grab is keyed by item identity and re-resolved against the
//!   owner's *current* rows at every use, so rows changing underneath
//!   (the playlist is replaced by every snapshot) can neither retarget
//!   the drag onto a neighbour nor index out of bounds.
//! - The pointer is hit-tested against the press-time geometry, and the
//!   owner renders with the press-time viewport center
//!   ([`DragReorder::center`]), so the rows on screen and the rows the
//!   pointer maps to cannot disagree mid-drag. Reach is therefore the
//!   rows visible at the press (design.md, Mouse support).

use std::ops::Range;

use crate::ui::layout::RenderedCollection;

/// The reorder a finished drag commits: `key` now directly follows
/// `after`, or heads its draggable run when `after` is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Move<K> {
    /// The dragged item.
    pub key: K,
    /// The item it now follows.
    pub after: Option<K>,
}

/// An armed drag (left button held after a press on a draggable row).
#[derive(Clone, Debug)]
pub struct DragReorder<K> {
    key: K,
    geometry: RenderedCollection,
    center: Option<usize>,
    movable: Range<usize>,
    origin: usize,
    target: usize,
}

impl<K: Clone + PartialEq> DragReorder<K> {
    /// Arm on a left press at `(column, row)`. `geometry` and `center`
    /// are what the owner's last render painted and centered on.
    /// `keys[i]` is the identity of row `i` when it may be dragged and
    /// `None` when it is fixed; a press on a fixed row (or on no row)
    /// arms nothing. The grabbed row may land anywhere within the
    /// contiguous run of draggable rows around it.
    pub fn grab(
        geometry: &RenderedCollection,
        column: u16,
        row: u16,
        center: Option<usize>,
        keys: &[Option<K>],
    ) -> Option<Self> {
        let origin = geometry.hit(column, row)?;
        let key = keys.get(origin)?.clone()?;
        let start = keys[..origin]
            .iter()
            .rposition(Option::is_none)
            .map_or(0, |fixed| fixed + 1);
        let end = keys[origin..]
            .iter()
            .position(Option::is_none)
            .map_or(keys.len(), |fixed| origin + fixed);
        Some(Self {
            key,
            geometry: geometry.clone(),
            center,
            movable: start..end,
            origin,
            target: origin,
        })
    }

    /// The viewport center to render with while the drag lasts.
    pub fn center(&self) -> Option<usize> {
        self.center
    }

    /// Follow the pointer to screen row `row` (the column is irrelevant:
    /// a drag is a grab). Returns whether the landing spot changed.
    pub fn drag(&mut self, row: u16) -> bool {
        let Some(index) = self.geometry.nearest_row(row) else {
            return false;
        };
        // `movable` contains the origin, so it is never empty.
        let target = index.clamp(self.movable.start, self.movable.end - 1);
        let changed = target != self.target;
        self.target = target;
        changed
    }

    /// Reorder `items` (the owner's current rows) the way the drag shows
    /// them: the grabbed item moved to the landing spot. Returns its
    /// index in the preview, or `None` (items untouched) when the
    /// grabbed item no longer exists. While the landing spot is the
    /// pressed row the items stay as they are: rows shifting under a
    /// motionless press must not turn it into a move.
    pub fn preview<T>(&self, items: &mut Vec<T>, key_of: impl Fn(&T) -> K) -> Option<usize> {
        let from = items.iter().position(|item| key_of(item) == self.key)?;
        if self.target == self.origin {
            return Some(from);
        }
        let item = items.remove(from);
        let to = self.target.min(items.len());
        items.insert(to, item);
        Some(to)
    }

    /// Finish the gesture against the owner's current rows: the one
    /// move to commit, or `None` when the pointer is back on the pressed
    /// row (a plain click — clicking never reorders), the drop changes
    /// nothing, or the row vanished mid-drag.
    pub fn release<T>(self, items: &[T], key_of: impl Fn(&T) -> K) -> Option<Move<K>> {
        let mut order: Vec<K> = items.iter().map(key_of).collect();
        let from = order.iter().position(|key| *key == self.key)?;
        let to = self.preview(&mut order, K::clone)?;
        if to == from {
            return None;
        }
        let after = (to > self.movable.start).then(|| order[to - 1].clone());
        Some(Move {
            key: self.key,
            after,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use proptest::prelude::*;
    use tuirealm::ratatui::layout::Rect;

    /// One painted row per index starting at screen row `y0`, covering
    /// `visible` (the frozen viewport).
    fn geometry(visible: Range<usize>, y0: u16) -> RenderedCollection {
        RenderedCollection::from_rows(
            visible
                .clone()
                .map(|index| {
                    let y = y0 + (index - visible.start) as u16;
                    (Rect::new(2, y, 20, 1), index)
                })
                .collect(),
        )
    }

    fn keys(n: u32) -> Vec<Option<u32>> {
        (0..n).map(Some).collect()
    }

    /// Apply a committed move the way the playlist CRDT does: remove the
    /// key, reinsert it after the anchor (or at the head).
    fn apply(items: &[u32], mv: &Move<u32>) -> Vec<u32> {
        let mut out: Vec<u32> = items.iter().copied().filter(|k| *k != mv.key).collect();
        let at = match mv.after {
            None => 0,
            Some(anchor) => out.iter().position(|k| *k == anchor).unwrap() + 1,
        };
        out.insert(at, mv.key);
        out
    }

    #[test]
    fn press_on_a_fixed_or_empty_spot_arms_nothing() {
        let rows = geometry(0..4, 1);
        let mut ks = keys(4);
        ks[2] = None;
        assert!(DragReorder::grab(&rows, 3, 3, None, &ks).is_none()); // fixed row
        assert!(DragReorder::grab(&rows, 3, 9, None, &ks).is_none()); // below the rows
        assert!(DragReorder::grab(&rows, 0, 1, None, &ks).is_none()); // left of them
    }

    #[test]
    fn drag_moves_within_its_run_and_releases_one_anchored_move() {
        let rows = geometry(0..5, 1);
        let items = [10, 11, 12, 13, 14];
        let ks: Vec<_> = items.iter().copied().map(Some).collect();
        let mut drag = DragReorder::grab(&rows, 3, 1, None, &ks).unwrap(); // grab 10
        assert!(drag.drag(3));
        assert!(!drag.drag(3), "same row, no change");
        let mut shown = items.to_vec();
        assert_eq!(drag.preview(&mut shown, |k| *k), Some(2));
        assert_eq!(shown, [11, 12, 10, 13, 14]);
        assert_eq!(
            drag.release(&items, |k| *k),
            Some(Move {
                key: 10,
                after: Some(12)
            })
        );
    }

    #[test]
    fn dragging_to_the_top_anchors_to_none_and_back_home_is_no_move() {
        let rows = geometry(0..3, 5);
        let items = [1, 2, 3];
        let ks: Vec<_> = items.iter().copied().map(Some).collect();
        let mut drag = DragReorder::grab(&rows, 3, 7, None, &ks).unwrap(); // grab 3
        drag.drag(0); // far above the pane: clamps to the first row
        assert_eq!(
            drag.clone().release(&items, |k| *k),
            Some(Move {
                key: 3,
                after: None
            })
        );
        drag.drag(7);
        assert_eq!(drag.release(&items, |k| *k), None);
    }

    #[test]
    fn a_fixed_row_bounds_the_run() {
        // Rows 0..2 fixed, 2..5 draggable, 5 fixed (an "Add" row).
        let rows = geometry(0..6, 0);
        let ks = [None, None, Some(2), Some(3), Some(4), None];
        let mut drag = DragReorder::grab(&rows, 3, 4, None, &ks).unwrap(); // grab 4
        drag.drag(0);
        let items = [0, 1, 2, 3, 4, 5];
        assert_eq!(
            drag.clone().release(&items, |k| *k),
            Some(Move {
                key: 4,
                after: None
            }),
            "landing on the first draggable row heads the run",
        );
        let mut shown = items.to_vec();
        drag.preview(&mut shown, |k| *k);
        assert_eq!(shown, [0, 1, 4, 2, 3, 5]);
        drag.drag(5);
        assert_eq!(
            drag.release(&items, |k| *k),
            None,
            "clamped above the fixed row"
        );
    }

    #[derive(Clone, Debug)]
    enum Step {
        Drag(u16),
        /// Another client removes the item at this index (mod len), unless
        /// it is the grabbed one.
        RemoveOther(usize),
        /// Another client inserts a fresh item at this index (mod len+1).
        Insert(usize),
    }

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![
            (0u16..40).prop_map(Step::Drag),
            (0usize..64).prop_map(Step::RemoveOther),
            (0usize..64).prop_map(Step::Insert),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(
            dessplay_core::test_support::proptest_cases(256)
        ))]

        /// Whatever the pointer does and however the list churns
        /// underneath (the synced playlist), release commits at most one
        /// move; that move, applied by identity to the current rows,
        /// reproduces exactly what the preview showed; and its anchor is
        /// a row that exists. A release where the preview equals the
        /// current order commits nothing.
        #[test]
        fn release_commits_exactly_the_previewed_order(
            len in 1u32..12,
            window_start in 0usize..12,
            window_len in 1usize..12,
            y0 in 0u16..10,
            press in 0usize..12,
            steps in prop::collection::vec(step(), 0..24),
            vanish in any::<bool>(),
        ) {
            let mut items: Vec<u32> = (0..len).collect();
            let start = window_start.min(len as usize - 1);
            let visible = start..(start + window_len).min(len as usize);
            let rows = geometry(visible.clone(), y0);
            let press = visible.start + press % visible.len();
            let screen = y0 + (press - visible.start) as u16;
            let mut drag = DragReorder::grab(&rows, 3, screen, Some(start), &keys(len))
                .expect("a press on a painted draggable row arms");
            let grabbed = items[press];
            let mut next = len;
            for step in steps {
                match step {
                    Step::Drag(row) => {
                        drag.drag(row);
                    }
                    Step::RemoveOther(i) => {
                        let i = i % items.len();
                        if items[i] != grabbed {
                            items.remove(i);
                        }
                    }
                    Step::Insert(i) => {
                        items.insert(i % (items.len() + 1), next);
                        next += 1;
                    }
                }
                let mut shown = items.clone();
                prop_assert!(drag.preview(&mut shown, |k| *k).is_some());
                let mut sorted = shown.clone();
                sorted.sort_unstable();
                let mut expect = items.clone();
                expect.sort_unstable();
                prop_assert_eq!(sorted, expect, "preview is a permutation");
            }
            if vanish {
                items.retain(|k| *k != grabbed);
                prop_assert_eq!(drag.release(&items, |k| *k), None);
                return Ok(());
            }
            let mut shown = items.clone();
            drag.preview(&mut shown, |k| *k);
            let stayed = drag.target == drag.origin;
            let released = drag.release(&items, |k| *k);
            if stayed {
                prop_assert_eq!(&released, &None, "a motionless press never moves");
            }
            match released {
                None => prop_assert_eq!(shown, items),
                Some(mv) => {
                    prop_assert_eq!(mv.key, grabbed);
                    if let Some(anchor) = mv.after {
                        prop_assert!(items.contains(&anchor));
                    }
                    prop_assert_ne!(&shown, &items);
                    prop_assert_eq!(apply(&items, &mv), shown);
                }
            }
        }
    }
}
