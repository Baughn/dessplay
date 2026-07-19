//! The one selection cursor. Every list in the UI — panes, browsers,
//! forms, search results — navigates through a [`ListCursor`], so the
//! movement vocabulary (Up/Down, PgUp/PgDn, clamping at the edges) is
//! identical everywhere by construction. "PgUp works in this pane but
//! not that one" was the observable drift this replaces.

use tuirealm::event::Key;
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::text::Line;
use tuirealm::ratatui::widgets::{Block, Borders, List, ListItem, ListState};

use crate::ui::theme;

/// How many rows PgUp/PgDown jump.
pub const PAGE_STEP: usize = 10;

/// A selection cursor over a list of rows. Pure state: the row count is
/// passed in per event, so the cursor can never hold an out-of-range
/// index the caller forgot to clamp.
#[derive(Clone, Copy, Debug, Default)]
pub struct ListCursor {
    sel: usize,
}

impl ListCursor {
    /// The selected row index.
    pub fn index(&self) -> usize {
        self.sel
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

/// Render the standard bordered, highlight-selected list — the one shape
/// every pane and modal list uses. `selected` is the caller's highlight
/// policy (panes highlight only while focused; modals always); `center` is
/// the independent viewport target.
pub fn render_list<'a>(
    frame: &mut Frame,
    area: Rect,
    title: impl Into<Line<'a>>,
    items: Vec<ListItem<'a>>,
    selected: Option<usize>,
    focused: bool,
    center: Option<usize>,
) {
    let visible = area.height.saturating_sub(2) as usize;
    let mut state = centered_state(items.len(), visible, selected, center);
    frame.render_stateful_widget(
        List::new(items)
            .highlight_style(theme::highlight_style())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme::border_style(focused))
                    .title(title),
            ),
        area,
        &mut state,
    );
}

/// Render a selectable list into an already-defined body area (no border).
/// Forms and search modals use this while bordered panes use [`render_list`];
/// both therefore share exactly the same viewport policy.
pub fn render_list_body<'a>(
    frame: &mut Frame,
    area: Rect,
    items: Vec<ListItem<'a>>,
    selected: Option<usize>,
    center: Option<usize>,
) {
    let mut state = centered_state(items.len(), area.height as usize, selected, center);
    frame.render_stateful_widget(
        List::new(items).highlight_style(theme::highlight_style()),
        area,
        &mut state,
    );
}

/// Ratatui selection/highlight state with an independent scroll target.
fn centered_state(
    len: usize,
    visible: usize,
    selected: Option<usize>,
    center: Option<usize>,
) -> ListState {
    let offset = center
        .map(|target| centered_offset(len, visible, target))
        .unwrap_or(0);
    ListState::default()
        .with_selected(selected)
        .with_offset(offset)
}

/// Map a click inside a bordered list (the [`render_list`] shape) to the
/// row index it landed on, reproducing the centered viewport the last
/// render used — so `len` and `center` must be the same values the pane
/// passed to [`render_list`]. `None` for a click on the border or past
/// the end of the list.
pub fn clicked_index(
    len: usize,
    area: Rect,
    center: Option<usize>,
    column: u16,
    row: u16,
) -> Option<usize> {
    // Strictly inside the border (the saturating_subs make a degenerate
    // zero-sized area a miss rather than an underflow).
    if column <= area.x
        || column >= area.x + area.width.saturating_sub(1)
        || row <= area.y
        || row >= area.y + area.height.saturating_sub(1)
    {
        return None;
    }
    let visible = area.height.saturating_sub(2) as usize;
    let offset = center
        .map(|target| centered_offset(len, visible, target))
        .unwrap_or(0);
    let index = offset + (row - area.y - 1) as usize;
    (index < len).then_some(index)
}

/// First visible row for a one-line-item viewport centered on `target`,
/// clamped so the viewport stays full near either edge.
fn centered_offset(len: usize, visible: usize, target: usize) -> usize {
    let max_offset = len.saturating_sub(visible);
    target
        .min(len.saturating_sub(1))
        .saturating_sub(visible / 2)
        .min(max_offset)
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

    #[test]
    fn centered_viewport_clamps_at_both_edges() {
        assert_eq!(centered_offset(20, 7, 10), 7);
        assert_eq!(centered_offset(20, 7, 1), 0);
        assert_eq!(centered_offset(20, 7, 19), 13);
        assert_eq!(centered_offset(5, 7, 3), 0);
    }

    #[test]
    fn clicked_index_maps_body_rows_and_rejects_borders() {
        // A 10x9 bordered list at (5, 3): body rows are y 4..=10.
        let area = Rect::new(5, 3, 10, 9);
        // Short list, no scrolling: body row N is index N.
        assert_eq!(clicked_index(5, area, Some(0), 6, 4), Some(0));
        assert_eq!(clicked_index(5, area, Some(0), 10, 7), Some(3));
        // Borders and outside are misses.
        assert_eq!(clicked_index(5, area, Some(0), 5, 4), None); // left border
        assert_eq!(clicked_index(5, area, Some(0), 14, 4), None); // right border
        assert_eq!(clicked_index(5, area, Some(0), 6, 3), None); // top border
        assert_eq!(clicked_index(5, area, Some(0), 6, 11), None); // bottom border
        assert_eq!(clicked_index(5, area, Some(0), 20, 20), None);
        // Past the end of a short list is a miss, not a clamp.
        assert_eq!(clicked_index(5, area, Some(0), 6, 10), None);
    }

    #[test]
    fn clicked_index_honors_the_centered_scroll() {
        // 7 visible rows of 20, centered on 10 => offset 7 (see above), so
        // the top body row is index 7.
        let area = Rect::new(0, 0, 10, 9);
        assert_eq!(clicked_index(20, area, Some(10), 1, 1), Some(7));
        assert_eq!(clicked_index(20, area, Some(10), 1, 7), Some(13));
        // No center (an unfocused playlist with nothing playing) pins the
        // viewport to the top.
        assert_eq!(clicked_index(20, area, None, 1, 1), Some(0));
    }

    #[test]
    fn clicked_index_on_a_degenerate_area_is_a_miss() {
        assert_eq!(clicked_index(5, Rect::new(0, 0, 0, 0), Some(0), 0, 0), None);
        assert_eq!(clicked_index(5, Rect::new(0, 0, 2, 2), Some(0), 1, 1), None);
    }
}
