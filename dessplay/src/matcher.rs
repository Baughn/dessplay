//! Minimal local-file matching, pulled forward from Phase 9 so a watch
//! party works end to end: when a playlist entry appears, find our copy
//! by **exact filename** under the media roots and verify its contents
//! against the entry's ed2k hash (design.md, File Matching).
//!
//! Everything here is synchronous and filesystem-bound — call it from
//! `spawn_blocking`. Phase 9's FileActor absorbs this module (adding
//! mtime tracking, manual mapping UI, downloads, and the cache).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use dessplay_core::hash::ed2k_hash_reader;
use dessplay_core::types::Ed2kHash;

/// What resolving a playlist entry against the media roots found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// A file with the right name *and* the right contents.
    Verified(PathBuf),
    /// Name matches somewhere, but no candidate's contents hash to the
    /// playlist key (a different encode — blocks playback, design.md
    /// "Before Playback Starts").
    HashMismatch(PathBuf),
    /// No file with that name under any root.
    NotFound,
}

/// Find a local copy of `filename` under `roots`, verified against
/// `expected`. Candidates are checked in root order (the first root is
/// the download target, most likely to be current).
pub fn resolve(filename: &str, roots: &[PathBuf], expected: Ed2kHash) -> Resolution {
    let mut mismatch = None;
    for candidate in find_by_name(filename, roots) {
        match std::fs::File::open(&candidate).and_then(ed2k_hash_reader) {
            Ok(hashed) if hashed.root == expected => {
                return Resolution::Verified(candidate);
            }
            Ok(_) => {
                tracing::debug!(path = %candidate.display(), "filename match, hash mismatch");
                mismatch.get_or_insert(candidate);
            }
            Err(e) => {
                tracing::debug!(path = %candidate.display(), "unreadable candidate: {e}");
            }
        }
    }
    match mismatch {
        Some(path) => Resolution::HashMismatch(path),
        None => Resolution::NotFound,
    }
}

/// Every file named exactly `filename` under the roots, in breadth-first
/// root order. Symlinked directories are skipped (cycle safety).
fn find_by_name(filename: &str, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in roots {
        let mut queue = VecDeque::from([root.clone()]);
        while let Some(dir) = queue.pop_front() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(e) => {
                    tracing::debug!(dir = %dir.display(), "unreadable directory: {e}");
                    continue;
                }
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    queue.push_back(path);
                } else if file_type.is_file() && entry.file_name().to_string_lossy() == filename {
                    found.push(path);
                }
            }
        }
    }
    found
}

/// Verify a specific path against the expected hash (used when loading
/// a previously-resolved file whose contents may have changed).
pub fn verify(path: &Path, expected: Ed2kHash) -> bool {
    std::fs::File::open(path)
        .and_then(ed2k_hash_reader)
        .map(|hashed| hashed.root == expected)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use dessplay_core::hash::ed2k_hash_bytes;

    fn write(dir: &Path, rel: &str, contents: &[u8]) -> PathBuf {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn finds_a_verified_file_in_a_nested_directory() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"episode one".as_slice();
        let path = write(root.path(), "Frieren/Season 1/ep1.mkv", contents);
        let expected = ed2k_hash_bytes(contents).root;
        assert_eq!(
            resolve("ep1.mkv", &[root.path().to_path_buf()], expected),
            Resolution::Verified(path)
        );
    }

    #[test]
    fn searches_later_roots_too() {
        let empty = tempfile::tempdir().unwrap();
        let full = tempfile::tempdir().unwrap();
        let contents = b"episode".as_slice();
        let path = write(full.path(), "ep1.mkv", contents);
        let expected = ed2k_hash_bytes(contents).root;
        assert_eq!(
            resolve(
                "ep1.mkv",
                &[empty.path().to_path_buf(), full.path().to_path_buf()],
                expected
            ),
            Resolution::Verified(path)
        );
    }

    #[test]
    fn wrong_contents_is_a_mismatch_not_a_match() {
        let root = tempfile::tempdir().unwrap();
        let path = write(root.path(), "ep1.mkv", b"a different encode");
        let expected = ed2k_hash_bytes(b"the real file").root;
        assert_eq!(
            resolve("ep1.mkv", &[root.path().to_path_buf()], expected),
            Resolution::HashMismatch(path)
        );
    }

    #[test]
    fn a_verified_copy_beats_an_earlier_mismatch() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"the real file".as_slice();
        // Same filename twice: a stale encode and the right copy.
        write(root.path(), "a/ep1.mkv", b"a different encode");
        let good = write(root.path(), "z/ep1.mkv", contents);
        let expected = ed2k_hash_bytes(contents).root;
        assert_eq!(
            resolve("ep1.mkv", &[root.path().to_path_buf()], expected),
            Resolution::Verified(good)
        );
    }

    #[test]
    fn absent_file_is_not_found() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "other.mkv", b"x");
        assert_eq!(
            resolve("ep1.mkv", &[root.path().to_path_buf()], Ed2kHash([0; 16])),
            Resolution::NotFound
        );
    }

    #[test]
    fn name_match_requires_exactness() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), "ep1.mkv.part", b"x");
        write(root.path(), "xep1.mkv", b"x");
        assert_eq!(
            resolve("ep1.mkv", &[root.path().to_path_buf()], Ed2kHash([0; 16])),
            Resolution::NotFound
        );
    }

    #[test]
    fn verify_checks_a_specific_path() {
        let root = tempfile::tempdir().unwrap();
        let contents = b"contents".as_slice();
        let path = write(root.path(), "ep1.mkv", contents);
        assert!(verify(&path, ed2k_hash_bytes(contents).root));
        assert!(!verify(&path, Ed2kHash([1; 16])));
        assert!(!verify(&root.path().join("missing.mkv"), Ed2kHash([1; 16])));
    }
}
