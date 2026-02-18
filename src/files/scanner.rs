use std::path::{Path, PathBuf};

/// Common video file extensions.
const MEDIA_EXTENSIONS: &[&str] = &["mkv", "mp4", "avi", "webm", "ogv", "m4v", "ts", "flv"];

/// Check if a filename has a recognized media file extension.
pub fn is_media_file(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    MEDIA_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
}

/// A media file found by scanning.
pub struct MediaFile {
    /// Basename only (e.g. "Frieren - 01.mkv").
    pub filename: String,
    /// Full absolute path to the file.
    pub full_path: PathBuf,
    /// Full path of the parent directory.
    pub parent_path: PathBuf,
}

/// A directory entry found by listing.
pub struct DirEntry {
    pub name: String,
    pub path: PathBuf,
}

/// Search all media roots recursively for an exact filename match.
/// Returns the first match found (roots are searched in order).
pub fn find_file(roots: &[String], filename: &str) -> Option<PathBuf> {
    for root in roots {
        if let Some(path) = find_in_directory(Path::new(root), filename) {
            return Some(path);
        }
    }
    None
}

fn find_in_directory(dir: &Path, target_filename: &str) -> Option<PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return None,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_in_directory(&path, target_filename) {
                return Some(found);
            }
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name == target_filename {
                return Some(path);
            }
        }
    }
    None
}

/// List subdirectories and media files in a directory (non-recursive).
/// Returns (directories sorted alphabetically, media files sorted alphabetically).
pub fn list_entries(path: &Path) -> (Vec<DirEntry>, Vec<MediaFile>) {
    let mut dirs = Vec::new();
    let mut files = Vec::new();

    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return (dirs, files),
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        // Skip hidden files/directories
        if name.starts_with('.') {
            continue;
        }
        if entry_path.is_dir() {
            dirs.push(DirEntry {
                name,
                path: entry_path,
            });
        } else if is_media_file(&name) {
            files.push(MediaFile {
                filename: name,
                full_path: entry_path.clone(),
                parent_path: path.to_path_buf(),
            });
        }
    }

    dirs.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
    files.sort_by(|a, b| {
        a.filename
            .to_ascii_lowercase()
            .cmp(&b.filename.to_ascii_lowercase())
    });

    (dirs, files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_media_file_checks_extensions() {
        assert!(is_media_file("video.mkv"));
        assert!(is_media_file("video.MP4"));
        assert!(is_media_file("show.avi"));
        assert!(is_media_file("clip.webm"));
        assert!(!is_media_file("readme.txt"));
        assert!(!is_media_file("image.png"));
        assert!(!is_media_file("subtitle.srt"));
        assert!(!is_media_file("noext"));
    }

    #[test]
    fn find_file_in_nested_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("series");
        std::fs::create_dir(&sub).unwrap();
        let file_path = sub.join("ep01.mkv");
        std::fs::write(&file_path, b"").unwrap();

        let roots = vec![dir.path().to_string_lossy().to_string()];
        let found = find_file(&roots, "ep01.mkv");
        assert_eq!(found, Some(file_path));

        assert!(find_file(&roots, "nonexistent.mkv").is_none());
    }

    #[test]
    fn find_file_searches_roots_in_order() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();

        let path1 = dir1.path().join("video.mkv");
        let path2 = dir2.path().join("video.mkv");
        std::fs::write(&path1, b"first").unwrap();
        std::fs::write(&path2, b"second").unwrap();

        let roots = vec![
            dir1.path().to_string_lossy().to_string(),
            dir2.path().to_string_lossy().to_string(),
        ];
        // Should find in first root
        assert_eq!(find_file(&roots, "video.mkv"), Some(path1));
    }

    #[test]
    fn list_entries_separates_dirs_and_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        std::fs::write(dir.path().join("video.mkv"), b"").unwrap();
        std::fs::write(dir.path().join("readme.txt"), b"").unwrap();
        std::fs::write(dir.path().join(".hidden.mkv"), b"").unwrap();

        let (dirs, files) = list_entries(dir.path());
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].name, "subdir");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "video.mkv");
    }
}
