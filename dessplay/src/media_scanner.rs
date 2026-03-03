//! Media root scanning — builds an index of filenames across media directories.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rayon::prelude::*;

use crate::storage::FileMappingEntry;

/// A media file discovered during a scan, with filesystem metadata.
#[derive(Clone, Debug)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub size: u64,
    pub mtime: SystemTime,
    pub dev_id: u64,
}

/// An index of media files found across all configured media roots.
///
/// Keyed by filename (case-sensitive), with values being indices into the
/// `files` vec. Multiple entries can exist if the same filename appears
/// in different directories.
#[derive(Clone, Debug, Default)]
pub struct MediaIndex {
    files: Vec<ScannedFile>,
    by_filename: HashMap<String, Vec<usize>>,
    by_path: HashMap<PathBuf, usize>,
}

impl MediaIndex {
    /// Recursively scan all `roots` and build a filename index.
    /// Subdirectories at each level are traversed in parallel via rayon.
    pub fn scan(roots: &[PathBuf]) -> Self {
        let all_files: Vec<ScannedFile> = roots
            .iter()
            .flat_map(|root| scan_dir_parallel(root))
            .collect();

        let mut by_filename: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_path: HashMap<PathBuf, usize> = HashMap::new();
        for (i, file) in all_files.iter().enumerate() {
            if let Some(name) = file.path.file_name() {
                let name = name.to_string_lossy().to_string();
                by_filename.entry(name).or_default().push(i);
            }
            by_path.insert(file.path.clone(), i);
        }

        Self {
            files: all_files,
            by_filename,
            by_path,
        }
    }

    /// Look up all paths matching a given filename.
    pub fn find_by_filename(&self, filename: &str) -> Option<Vec<&Path>> {
        self.by_filename.get(filename).map(|indices| {
            indices
                .iter()
                .map(|&i| self.files[i].path.as_path())
                .collect()
        })
    }

    /// Returns every indexed media file path.
    pub fn all_paths(&self) -> Vec<&Path> {
        self.files.iter().map(|f| f.path.as_path()).collect()
    }

    /// Iterator over all scanned files with metadata.
    pub fn all_scanned_files(&self) -> impl Iterator<Item = &ScannedFile> {
        self.files.iter()
    }

    /// Look up a scanned file by its exact path.
    pub fn get_by_path(&self, path: &Path) -> Option<&ScannedFile> {
        self.by_path.get(path).map(|&i| &self.files[i])
    }

    /// Total number of indexed files.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

/// Recursively scan a directory in parallel using rayon.
fn scan_dir_parallel(dir: &Path) -> Vec<ScannedFile> {
    let entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.flatten().collect(),
        Err(e) => {
            tracing::debug!(dir = %dir.display(), "Failed to read directory: {e}");
            return vec![];
        }
    };

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }

        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if file_type.is_dir() {
            dirs.push(path);
        } else if file_type.is_file() && is_media_file(&name) {
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            files.push(ScannedFile {
                path,
                size: meta.len(),
                mtime: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                dev_id: meta.dev(),
            });
        }
    }

    let sub_files: Vec<ScannedFile> = dirs
        .par_iter()
        .flat_map(|d| scan_dir_parallel(d))
        .collect();

    files.extend(sub_files);
    files
}

/// Result of comparing a fresh scan against stored file mappings.
pub struct RescanDiff {
    /// Files that are new or have changed mtime/size since last hash.
    pub files_to_hash: Vec<PathBuf>,
    /// Stored mappings whose paths no longer exist on disk (and whose
    /// media root directory still exists).
    pub stale_paths: Vec<PathBuf>,
}

/// Compare a fresh `MediaIndex` against stored `FileMappingEntry` rows
/// to find files needing (re-)hashing and stale mappings to clean up.
pub fn compute_rescan_diff(
    index: &MediaIndex,
    stored: &[FileMappingEntry],
    media_roots: &[PathBuf],
) -> RescanDiff {
    // Build a lookup from path → stored entry
    let stored_by_path: HashMap<&Path, &FileMappingEntry> =
        stored.iter().map(|e| (e.local_path.as_path(), e)).collect();

    // Scanned paths for stale detection
    let scanned_paths: HashSet<&Path> = index.files.iter().map(|f| f.path.as_path()).collect();

    // Files to hash: new or changed
    let mut files_to_hash = Vec::new();
    for file in &index.files {
        match stored_by_path.get(file.path.as_path()) {
            None => {
                // New file, not yet hashed
                files_to_hash.push(file.path.clone());
            }
            Some(entry) => {
                let mtime_dur = file
                    .mtime
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default();
                let stored_secs = entry.mtime_secs;
                let stored_nanos = entry.mtime_nanos;
                if mtime_dur.as_secs() as i64 != stored_secs
                    || mtime_dur.subsec_nanos() != stored_nanos
                    || file.size != entry.file_size
                {
                    files_to_hash.push(file.path.clone());
                }
            }
        }
    }

    // Stale paths: in stored but not in scan, and root dir still exists
    let stale_paths: Vec<PathBuf> = stored
        .iter()
        .filter(|entry| {
            if scanned_paths.contains(entry.local_path.as_path()) {
                return false;
            }
            // Only consider stale if the media root exists and is non-empty
            media_roots.iter().any(|root| {
                entry.local_path.starts_with(root)
                    && root.is_dir()
                    && root.read_dir().is_ok_and(|mut d| d.next().is_some())
            })
        })
        .map(|entry| entry.local_path.clone())
        .collect();

    RescanDiff {
        files_to_hash,
        stale_paths,
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
    use dessplay_core::types::FileId;
    use std::fs;

    fn setup_temp_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();

        // Create some media files
        fs::write(dir.path().join("episode01.mkv"), b"data1").unwrap();
        fs::write(dir.path().join("episode02.mp4"), b"data2").unwrap();
        fs::write(dir.path().join("readme.txt"), b"text").unwrap();
        fs::write(dir.path().join(".hidden.mkv"), b"hidden").unwrap();

        // Create a subdirectory with files
        let sub = dir.path().join("Season 1");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("s01e01.mkv"), b"s1e1").unwrap();
        fs::write(sub.join("s01e02.mkv"), b"s1e2").unwrap();

        dir
    }

    fn fid(n: u8) -> FileId {
        let mut id = [0u8; 16];
        id[0] = n;
        FileId(id)
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
        fs::write(sub1.join("same.mkv"), b"a").unwrap();
        fs::write(sub2.join("same.mkv"), b"b").unwrap();

        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);
        let paths = index.find_by_filename("same.mkv").unwrap();
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn find_missing_returns_none() {
        let index = MediaIndex::scan(&[]);
        assert!(index.find_by_filename("nonexistent.mkv").is_none());
    }

    #[test]
    fn scanned_files_have_metadata() {
        let dir = setup_temp_dir();
        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);

        for file in index.all_scanned_files() {
            assert!(file.size > 0 || file.path.display().to_string().contains("episode"));
            // mtime should be recent (not UNIX_EPOCH)
            assert!(file.mtime > SystemTime::UNIX_EPOCH);
        }
    }

    #[test]
    fn rescan_diff_new_file() {
        let dir = setup_temp_dir();
        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);
        // No stored entries → all files need hashing
        let diff = compute_rescan_diff(&index, &[], &[dir.path().to_path_buf()]);
        assert_eq!(diff.files_to_hash.len(), 4);
        assert!(diff.stale_paths.is_empty());
    }

    #[test]
    fn rescan_diff_unchanged_file() {
        let dir = setup_temp_dir();
        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);

        // Simulate stored entries matching all scanned files
        let stored: Vec<FileMappingEntry> = index
            .all_scanned_files()
            .map(|f| {
                let dur = f.mtime.duration_since(SystemTime::UNIX_EPOCH).unwrap();
                FileMappingEntry {
                    local_path: f.path.clone(),
                    file_hash: fid(0),
                    file_size: f.size,
                    mtime_secs: dur.as_secs() as i64,
                    mtime_nanos: dur.subsec_nanos(),
                }
            })
            .collect();

        let diff = compute_rescan_diff(&index, &stored, &[dir.path().to_path_buf()]);
        assert!(diff.files_to_hash.is_empty());
        assert!(diff.stale_paths.is_empty());
    }

    #[test]
    fn rescan_diff_changed_mtime() {
        let dir = setup_temp_dir();
        let path = dir.path().join("episode01.mkv");

        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);
        let file = index
            .all_scanned_files()
            .find(|f| f.path == path)
            .unwrap();

        // Store with different mtime
        let stored = vec![FileMappingEntry {
            local_path: path.clone(),
            file_hash: fid(1),
            file_size: file.size,
            mtime_secs: 0,
            mtime_nanos: 0,
        }];

        let diff = compute_rescan_diff(&index, &stored, &[dir.path().to_path_buf()]);
        assert!(diff.files_to_hash.contains(&path));
    }

    #[test]
    fn rescan_diff_changed_size() {
        let dir = setup_temp_dir();
        let path = dir.path().join("episode01.mkv");

        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);
        let file = index
            .all_scanned_files()
            .find(|f| f.path == path)
            .unwrap();
        let dur = file
            .mtime
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap();

        // Store with different size
        let stored = vec![FileMappingEntry {
            local_path: path.clone(),
            file_hash: fid(1),
            file_size: file.size + 100,
            mtime_secs: dur.as_secs() as i64,
            mtime_nanos: dur.subsec_nanos(),
        }];

        let diff = compute_rescan_diff(&index, &stored, &[dir.path().to_path_buf()]);
        assert!(diff.files_to_hash.contains(&path));
    }

    #[test]
    fn rescan_diff_stale_path_root_present() {
        let dir = setup_temp_dir();
        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);

        // A stored entry for a file not on disk
        let stored = vec![FileMappingEntry {
            local_path: dir.path().join("deleted_episode.mkv"),
            file_hash: fid(99),
            file_size: 100,
            mtime_secs: 0,
            mtime_nanos: 0,
        }];

        let diff = compute_rescan_diff(&index, &stored, &[dir.path().to_path_buf()]);
        assert_eq!(diff.stale_paths.len(), 1);
        assert!(diff.stale_paths[0].ends_with("deleted_episode.mkv"));
    }

    #[test]
    fn rescan_diff_stale_path_root_missing() {
        let index = MediaIndex::scan(&[]);
        let missing_root = PathBuf::from("/nonexistent/root/12345");

        // Stored entry under a root that doesn't exist → NOT stale
        let stored = vec![FileMappingEntry {
            local_path: missing_root.join("file.mkv"),
            file_hash: fid(99),
            file_size: 100,
            mtime_secs: 0,
            mtime_nanos: 0,
        }];

        let diff = compute_rescan_diff(&index, &stored, &[missing_root]);
        assert!(diff.stale_paths.is_empty());
    }

    #[test]
    fn rescan_diff_stale_path_root_empty() {
        let dir = tempfile::tempdir().unwrap();
        // Root exists but is empty → NOT stale (e.g. NAS not mounted)
        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);

        let stored = vec![FileMappingEntry {
            local_path: dir.path().join("file.mkv"),
            file_hash: fid(99),
            file_size: 100,
            mtime_secs: 0,
            mtime_nanos: 0,
        }];

        let diff = compute_rescan_diff(&index, &stored, &[dir.path().to_path_buf()]);
        assert!(diff.stale_paths.is_empty());
    }

    #[test]
    fn scanned_files_have_nonzero_dev_id() {
        let dir = setup_temp_dir();
        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);
        for file in index.all_scanned_files() {
            assert!(file.dev_id > 0, "dev_id should be non-zero");
        }
    }

    #[test]
    fn get_by_path_returns_correct_file() {
        let dir = setup_temp_dir();
        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);
        let target = dir.path().join("episode01.mkv");
        let found = index.get_by_path(&target);
        assert!(found.is_some());
        assert_eq!(found.unwrap().path, target);
    }

    #[test]
    fn get_by_path_returns_none_for_missing() {
        let dir = setup_temp_dir();
        let index = MediaIndex::scan(&[dir.path().to_path_buf()]);
        assert!(index.get_by_path(Path::new("/nonexistent.mkv")).is_none());
    }
}
