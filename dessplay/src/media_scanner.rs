//! Media root scanning — builds an index of filenames across media directories.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// An index of media files found across all configured media roots.
///
/// Keyed by filename (case-sensitive), with values being all paths where that
/// filename was found. Multiple entries can exist if the same filename appears
/// in different directories.
#[derive(Clone, Debug, Default)]
pub struct MediaIndex {
    by_filename: HashMap<String, Vec<PathBuf>>,
}

impl MediaIndex {
    /// Recursively scan all `roots` and build a filename index.
    pub fn scan(roots: &[PathBuf]) -> Self {
        let mut index = Self::default();
        for root in roots {
            index.scan_dir(root);
        }
        index
    }

    /// Look up all paths matching a given filename.
    pub fn find_by_filename(&self, filename: &str) -> Option<&[PathBuf]> {
        self.by_filename.get(filename).map(|v| v.as_slice())
    }

    /// Total number of indexed files.
    pub fn file_count(&self) -> usize {
        self.by_filename.values().map(|v| v.len()).sum()
    }

    fn scan_dir(&mut self, dir: &Path) {
        let read_dir = match std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(e) => {
                tracing::debug!(dir = %dir.display(), "Failed to read directory: {e}");
                return;
            }
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            // Skip hidden files/directories
            if name.starts_with('.') {
                continue;
            }

            if path.is_dir() {
                self.scan_dir(&path);
            } else if is_media_file(&name) {
                self.by_filename
                    .entry(name)
                    .or_default()
                    .push(path);
            }
        }
    }
}

/// Check if a filename looks like a media file.
pub fn is_media_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".mkv")
        || lower.ends_with(".mp4")
        || lower.ends_with(".avi")
        || lower.ends_with(".webm")
        || lower.ends_with(".m4v")
        || lower.ends_with(".mov")
        || lower.ends_with(".wmv")
        || lower.ends_with(".flv")
        || lower.ends_with(".ogm")
        || lower.ends_with(".ts")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_temp_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();

        // Create some media files
        fs::write(dir.path().join("episode01.mkv"), b"").unwrap();
        fs::write(dir.path().join("episode02.mp4"), b"").unwrap();
        fs::write(dir.path().join("readme.txt"), b"").unwrap();
        fs::write(dir.path().join(".hidden.mkv"), b"").unwrap();

        // Create a subdirectory with files
        let sub = dir.path().join("Season 1");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("s01e01.mkv"), b"").unwrap();
        fs::write(sub.join("s01e02.mkv"), b"").unwrap();

        dir
    }

    #[test]
    fn scan_finds_media_files() {
        let dir = setup_temp_dir();
        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);

        assert!(index.find_by_filename("episode01.mkv").is_some());
        assert!(index.find_by_filename("episode02.mp4").is_some());
        assert_eq!(index.find_by_filename("episode01.mkv").unwrap().len(), 1);
    }

    #[test]
    fn scan_excludes_non_media() {
        let dir = setup_temp_dir();
        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);

        assert!(index.find_by_filename("readme.txt").is_none());
    }

    #[test]
    fn scan_excludes_hidden() {
        let dir = setup_temp_dir();
        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);

        assert!(index.find_by_filename(".hidden.mkv").is_none());
    }

    #[test]
    fn scan_recurses_into_subdirs() {
        let dir = setup_temp_dir();
        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);

        assert!(index.find_by_filename("s01e01.mkv").is_some());
        assert!(index.find_by_filename("s01e02.mkv").is_some());
    }

    #[test]
    fn scan_counts_correctly() {
        let dir = setup_temp_dir();
        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);

        // episode01.mkv, episode02.mp4, s01e01.mkv, s01e02.mkv = 4 files
        assert_eq!(index.file_count(), 4);
    }

    #[test]
    fn scan_empty_roots() {
        let index = MediaIndex::scan(&[]);
        assert_eq!(index.file_count(), 0);
    }

    #[test]
    fn scan_nonexistent_root() {
        let index = MediaIndex::scan(&[PathBuf::from("/nonexistent/path/12345")]);
        assert_eq!(index.file_count(), 0);
    }

    #[test]
    fn scan_duplicate_filenames() {
        let dir = tempfile::tempdir().unwrap();
        let sub1 = dir.path().join("dir1");
        let sub2 = dir.path().join("dir2");
        fs::create_dir(&sub1).unwrap();
        fs::create_dir(&sub2).unwrap();
        fs::write(sub1.join("same.mkv"), b"").unwrap();
        fs::write(sub2.join("same.mkv"), b"").unwrap();

        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);
        let paths = index.find_by_filename("same.mkv").unwrap();
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn find_missing_returns_none() {
        let index = MediaIndex::scan(&[]);
        assert!(index.find_by_filename("nonexistent.mkv").is_none());
    }
}
