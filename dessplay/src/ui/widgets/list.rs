//! The one selection cursor. Every list in the UI — panes, browsers,
//! forms, search results — navigates through a [`ListCursor`], so the
//! movement vocabulary (Up/Down, PgUp/PgDn, clamping at the edges) is
//! identical everywhere by construction. "PgUp works in this pane but
//! not that one" was the observable drift this replaces.

use tuirealm::event::Key;
/// How many rows PgUp/PgDown jump.
pub const PAGE_STEP: usize = 10;

/// A selection cursor over a list of rows. Pure state: the row count is
/// passed in per event, so the cursor can never hold an out-of-range
/// index the caller forgot to clamp.
#[derive(Clone, Debug, Default)]
pub struct ListCursor {
    sel: usize,
    hidden: Vec<usize>,
}

impl ListCursor {
    /// The selected row index.
    pub fn index(&self) -> usize {
        self.sel
    }

    /// The selected row may be absent when authors hide every choice.
    pub fn visible_index(&self) -> Option<usize> {
        self.hidden
            .binary_search(&self.sel)
            .is_err()
            .then_some(self.sel)
    }
    /// Place the cursor on a specific row.
    pub fn set(&mut self, sel: usize) {
        self.sel = sel;
    }

    /// Move the cursor back to the first row.
    pub fn reset(&mut self) {
        self.sel = 0;
    }

    /// Re-clamp after the row count changed (props replaced).
    pub fn clamp(&mut self, len: usize) {
        self.sel = self.sel.min(len.saturating_sub(1));
    }

    /// Handle a navigation key over `len` rows; returns whether the key
    /// was one of ours (Up/Down/PgUp/PgDn). Movement clamps at the ends
    /// (no wrap-around).
    pub fn nav(&mut self, key: Key, len: usize) -> bool {
        let hidden = std::mem::take(&mut self.hidden);
        let handled = self.nav_visible(key, len, &hidden);
        self.hidden = hidden;
        handled
    }

    /// Update explicit visibility from the renderer, independently of clipping.
    pub(crate) fn set_hidden(&mut self, hidden: &[usize]) {
        self.hidden = hidden.to_vec();
    }

    /// A hidden enclosing viewport has no actionable choices.
    pub(crate) fn hide_all(&mut self, len: usize) {
        self.hidden = (0..len).collect();
    }

    fn nav_unfiltered(&mut self, key: Key, len: usize) -> bool {
        let (down, delta) = match key {
            Key::Up => (false, 1),
            Key::Down => (true, 1),
            Key::PageUp => (false, PAGE_STEP),
            Key::PageDown => (true, PAGE_STEP),
            _ => return false,
        };
        self.sel = step_by(self.sel, len, down, delta);
        true
    }

    /// Move through authored-visible rows; clipping never enters this filter.
    pub fn nav_visible(&mut self, key: Key, len: usize, hidden: &[usize]) -> bool {
        if hidden.is_empty() {
            return self.nav_unfiltered(key, len);
        }
        let visible = (0..len)
            .filter(|index| hidden.binary_search(index).is_err())
            .collect::<Vec<_>>();
        let mut ordinal = Self {
            sel: visible
                .partition_point(|index| *index < self.sel)
                .min(visible.len().saturating_sub(1)),
            ..Default::default()
        };
        if !ordinal.nav_unfiltered(key, visible.len()) {
            return false;
        }
        self.sel = visible.get(ordinal.index()).copied().unwrap_or(0);
        true
    }

    /// Keep a surviving selection, otherwise choose the next authored-visible row.
    pub fn reconcile_visible(&mut self, len: usize, hidden: &[usize]) {
        self.sel = (self.sel..len)
            .find(|index| hidden.binary_search(index).is_err())
            .or_else(|| {
                (0..self.sel.min(len))
                    .rev()
                    .find(|index| hidden.binary_search(index).is_err())
            })
            .unwrap_or(0);
    }
}

/// Selection cursor over `len` rows, moved by `delta` and clamped at the
/// ends.
pub fn step_by(sel: usize, len: usize, down: bool, delta: usize) -> usize {
    if len == 0 {
        return 0;
    }
    if down {
        (sel + delta).min(len - 1)
    } else {
        sel.saturating_sub(delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_moves_and_clamps() {
        let mut c = ListCursor::default();
        assert!(c.nav(Key::Down, 5));
        assert_eq!(c.index(), 1);
        assert!(c.nav(Key::PageDown, 5));
        assert_eq!(c.index(), 4); // clamped to the last row
        assert!(c.nav(Key::Down, 5));
        assert_eq!(c.index(), 4); // stays
        assert!(c.nav(Key::PageUp, 5));
        assert_eq!(c.index(), 0);
        assert!(c.nav(Key::Up, 5));
        assert_eq!(c.index(), 0); // stays
        // Non-navigation keys are not consumed.
        assert!(!c.nav(Key::Enter, 5));
        assert!(!c.nav(Key::Char('a'), 5));
    }

    #[test]
    fn nav_on_an_empty_list_pins_to_zero() {
        let mut c = ListCursor::default();
        c.set(3);
        assert!(c.nav(Key::Down, 0));
        assert_eq!(c.index(), 0);
    }

    #[test]
    fn clamp_after_shrink() {
        let mut c = ListCursor::default();
        c.set(9);
        c.clamp(4);
        assert_eq!(c.index(), 3);
        c.clamp(0);
        assert_eq!(c.index(), 0);
    }
}
