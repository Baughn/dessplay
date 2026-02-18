use std::path::PathBuf;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};

use crate::files::scanner;
use crate::storage::Database;

/// An item displayed in the file browser.
enum BrowserItem {
    ParentDir,
    Directory { name: String, path: PathBuf },
    File { filename: String, full_path: PathBuf, watched: bool },
}

/// The result of selecting a file in the browser.
pub struct SelectedFile {
    pub filename: String,
    pub full_path: PathBuf,
}

/// Modal file browser overlay.
pub struct FileBrowserModal {
    current_dir: PathBuf,
    items: Vec<BrowserItem>,
    selected: usize,
    scroll: usize,
}

impl FileBrowserModal {
    /// Create a new file browser starting at the given directory.
    pub fn new(start_dir: PathBuf, db: &Database) -> Self {
        let mut modal = Self {
            current_dir: start_dir,
            items: Vec::new(),
            selected: 0,
            scroll: 0,
        };
        modal.refresh(db);
        modal
    }

    /// Refresh the item list by reading the filesystem.
    fn refresh(&mut self, db: &Database) {
        self.items.clear();

        // Always show parent directory entry (unless at root)
        if self.current_dir.parent().is_some() {
            self.items.push(BrowserItem::ParentDir);
        }

        let (dirs, files) = scanner::list_entries(&self.current_dir);

        // Directories first
        for dir in dirs {
            self.items.push(BrowserItem::Directory {
                name: dir.name,
                path: dir.path,
            });
        }

        // Then files, with unwatched before watched
        let mut unwatched = Vec::new();
        let mut watched = Vec::new();
        for file in files {
            let is_watched = db.is_watched(&file.filename).unwrap_or(false);
            if is_watched {
                watched.push(file);
            } else {
                unwatched.push(file);
            }
        }
        for file in unwatched {
            self.items.push(BrowserItem::File {
                filename: file.filename,
                full_path: file.full_path,
                watched: false,
            });
        }
        for file in watched {
            self.items.push(BrowserItem::File {
                filename: file.filename,
                full_path: file.full_path,
                watched: true,
            });
        }

        // Clamp selection
        if self.selected >= self.items.len() {
            self.selected = self.items.len().saturating_sub(1);
        }
        self.scroll = 0;
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if !self.items.is_empty() && self.selected < self.items.len() - 1 {
            self.selected += 1;
        }
    }

    /// Handle Enter: navigate into directory, or return selected file.
    pub fn enter(&mut self, db: &Database) -> Option<SelectedFile> {
        let item = self.items.get(self.selected)?;
        match item {
            BrowserItem::ParentDir => {
                self.go_up(db);
                None
            }
            BrowserItem::Directory { path, .. } => {
                self.current_dir = path.clone();
                self.selected = 0;
                self.scroll = 0;
                self.refresh(db);
                None
            }
            BrowserItem::File {
                filename,
                full_path,
                ..
            } => Some(SelectedFile {
                filename: filename.clone(),
                full_path: full_path.clone(),
            }),
        }
    }

    /// Navigate to parent directory.
    pub fn go_up(&mut self, db: &Database) {
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            self.selected = 0;
            self.scroll = 0;
            self.refresh(db);
        }
    }

    /// Adjust scroll so selected item is visible within the given height.
    fn ensure_visible(&mut self, visible_height: usize) {
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

/// Render the file browser modal as a centered overlay.
pub fn render(frame: &mut Frame, modal: &mut FileBrowserModal) {
    let area = frame.area();
    let modal_width = ((area.width as f32) * 0.70).max(40.0) as u16;
    let modal_height = ((area.height as f32) * 0.80).max(10.0) as u16;
    let modal_width = modal_width.min(area.width);
    let modal_height = modal_height.min(area.height);
    let x = (area.width.saturating_sub(modal_width)) / 2;
    let y = (area.height.saturating_sub(modal_height)) / 2;
    let modal_area = Rect::new(x, y, modal_width, modal_height);

    // Clear the area behind the modal
    frame.render_widget(Clear, modal_area);

    let title = format!(" {} ", modal.current_dir.display());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Yellow));

    let inner_height = modal_area.height.saturating_sub(2) as usize;
    modal.ensure_visible(inner_height);

    if modal.items.is_empty() {
        let paragraph = Paragraph::new("  (empty directory)")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(paragraph, modal_area);
        return;
    }

    let items: Vec<ListItem> = modal
        .items
        .iter()
        .enumerate()
        .skip(modal.scroll)
        .take(inner_height)
        .map(|(i, item)| {
            let is_selected = i == modal.selected;
            let prefix = if is_selected { "> " } else { "  " };
            let (text, mut style) = match item {
                BrowserItem::ParentDir => ("..".to_string(), Style::default().fg(Color::Blue)),
                BrowserItem::Directory { name, .. } => {
                    (format!("{name}/"), Style::default().fg(Color::Blue))
                }
                BrowserItem::File { filename, watched, .. } => {
                    let color = if *watched { Color::DarkGray } else { Color::White };
                    (filename.clone(), Style::default().fg(color))
                }
            };
            if is_selected {
                style = style.add_modifier(Modifier::BOLD);
            }
            ListItem::new(Line::from(Span::styled(format!("{prefix}{text}"), style)))
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, modal_area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_navigation() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(dir.path().join("video.mkv"), b"").unwrap();
        std::fs::write(dir.path().join("readme.txt"), b"").unwrap();

        let db = Database::open_in_memory().unwrap();
        let mut modal = FileBrowserModal::new(dir.path().to_path_buf(), &db);

        // Should have: ParentDir, subdir/, video.mkv (no readme.txt — not media)
        assert_eq!(modal.items.len(), 3);
        assert!(matches!(modal.items[0], BrowserItem::ParentDir));
        assert!(matches!(modal.items[1], BrowserItem::Directory { .. }));
        assert!(matches!(modal.items[2], BrowserItem::File { .. }));

        // Navigate into subdir
        modal.selected = 1;
        let result = modal.enter(&db);
        assert!(result.is_none()); // entered directory
        assert_eq!(modal.current_dir, sub);

        // Go back up
        modal.go_up(&db);
        assert_eq!(modal.current_dir, dir.path());
    }

    #[test]
    fn browser_select_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("episode.mkv");
        std::fs::write(&file_path, b"").unwrap();

        let db = Database::open_in_memory().unwrap();
        let mut modal = FileBrowserModal::new(dir.path().to_path_buf(), &db);

        // Select the file (index 1, after ParentDir)
        modal.selected = 1;
        let result = modal.enter(&db);
        assert!(result.is_some());
        let selected = result.unwrap();
        assert_eq!(selected.filename, "episode.mkv");
        assert_eq!(selected.full_path, file_path);
    }

    #[test]
    fn watched_files_sorted_after_unwatched() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a_first.mkv"), b"").unwrap();
        std::fs::write(dir.path().join("b_second.mkv"), b"").unwrap();
        std::fs::write(dir.path().join("c_third.mkv"), b"").unwrap();

        let db = Database::open_in_memory().unwrap();
        db.mark_watched("a_first.mkv", dir.path().to_str().unwrap())
            .unwrap();

        let modal = FileBrowserModal::new(dir.path().to_path_buf(), &db);

        // ParentDir, then unwatched (b_second, c_third), then watched (a_first)
        assert_eq!(modal.items.len(), 4); // ParentDir + 3 files
        match &modal.items[1] {
            BrowserItem::File { filename, watched, .. } => {
                assert_eq!(filename, "b_second.mkv");
                assert!(!watched);
            }
            _ => panic!("expected unwatched file"),
        }
        match &modal.items[3] {
            BrowserItem::File { filename, watched, .. } => {
                assert_eq!(filename, "a_first.mkv");
                assert!(watched);
            }
            _ => panic!("expected watched file"),
        }
    }
}
