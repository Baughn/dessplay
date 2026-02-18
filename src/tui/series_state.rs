use std::path::{Path, PathBuf};

use crate::storage::Database;

/// An item displayed in the Recent Series pane.
pub enum SeriesItem {
    MediaRoot { path: String },
    RecentSeries { directory: String, display_name: String },
}

impl SeriesItem {
    /// The directory path to browse when this item is selected.
    pub fn browse_path(&self) -> PathBuf {
        match self {
            SeriesItem::MediaRoot { path } => PathBuf::from(path),
            SeriesItem::RecentSeries { directory, .. } => PathBuf::from(directory),
        }
    }
}

pub struct SeriesPaneState {
    pub items: Vec<SeriesItem>,
    pub selected: usize,
    pub scroll: usize,
    pub dirty: bool,
}

impl SeriesPaneState {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            selected: 0,
            scroll: 0,
            dirty: true,
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            if self.selected < self.scroll {
                self.scroll = self.selected;
            }
        }
    }

    pub fn move_down(&mut self) {
        if !self.items.is_empty() && self.selected < self.items.len() - 1 {
            self.selected += 1;
        }
    }

    pub fn selected_item(&self) -> Option<&SeriesItem> {
        self.items.get(self.selected)
    }

    /// Refresh the item list from the database.
    pub fn refresh(&mut self, db: &Database) {
        let mut items = Vec::new();

        if let Ok(roots) = db.list_media_roots() {
            for root in roots {
                items.push(SeriesItem::MediaRoot { path: root });
            }
        }

        if let Ok(series) = db.recent_series() {
            for entry in series {
                let display_name = Path::new(&entry.directory)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                items.push(SeriesItem::RecentSeries {
                    directory: entry.directory,
                    display_name,
                });
            }
        }

        self.items = items;
        self.dirty = false;
        // Clamp selection
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
    }

    /// Adjust scroll so that the selected item is visible.
    pub fn ensure_visible(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible_height {
            self.scroll = self.selected + 1 - visible_height;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_up_at_top_stays() {
        let mut state = SeriesPaneState::new();
        state.items.push(SeriesItem::MediaRoot {
            path: "/a".into(),
        });
        state.items.push(SeriesItem::MediaRoot {
            path: "/b".into(),
        });
        state.selected = 0;
        state.move_up();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn move_down_at_bottom_stays() {
        let mut state = SeriesPaneState::new();
        state.items.push(SeriesItem::MediaRoot {
            path: "/a".into(),
        });
        state.items.push(SeriesItem::MediaRoot {
            path: "/b".into(),
        });
        state.selected = 1;
        state.move_down();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn move_down_advances() {
        let mut state = SeriesPaneState::new();
        state.items.push(SeriesItem::MediaRoot {
            path: "/a".into(),
        });
        state.items.push(SeriesItem::MediaRoot {
            path: "/b".into(),
        });
        state.selected = 0;
        state.move_down();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn empty_state_navigation() {
        let mut state = SeriesPaneState::new();
        state.move_up();
        state.move_down();
        assert_eq!(state.selected, 0);
        assert!(state.selected_item().is_none());
    }
}
