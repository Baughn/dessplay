use std::collections::HashMap;

use dessplay_core::crdt::CrdtState;
use dessplay_core::types::FileId;

use crate::storage::ClientStorage;

/// A series entry for the Recent Series pane.
#[derive(Clone, Debug)]
pub struct SeriesEntry {
    pub anime_id: u64,
    pub name: String,
    pub has_unwatched: bool,
    pub last_watched_at: Option<u64>,
}

/// Build the list of series for the Recent Series pane.
///
/// A series appears if it has AniDB metadata and at least one locally-mapped file.
/// Sort: unwatched first → most recently watched → alphabetical.
pub fn build_series_list(crdt: &CrdtState, storage: &ClientStorage) -> Vec<SeriesEntry> {
    let watched: HashMap<FileId, u64> = storage
        .watched_files()
        .unwrap_or_default()
        .into_iter()
        .collect();

    // Group files by anime_id.
    // Value: (name, has_unwatched_local_file, max_watched_timestamp)
    let mut series_map: HashMap<u64, (String, bool, Option<u64>)> = HashMap::new();

    for (file_id, (_ts, value)) in crdt.anidb.iter() {
        if let Some(meta) = value {
            let has_local = storage
                .get_file_mapping(file_id)
                .ok()
                .flatten()
                .is_some();

            if !has_local {
                continue;
            }

            let entry = series_map
                .entry(meta.anime_id)
                .or_insert_with(|| (meta.anime_name.clone(), false, None));

            if let Some(ts) = watched.get(file_id) {
                entry.2 = Some(entry.2.map_or(*ts, |prev| prev.max(*ts)));
            } else {
                entry.1 = true;
            }
        }
    }

    let mut entries: Vec<SeriesEntry> = series_map
        .into_iter()
        .map(|(anime_id, (name, has_unwatched, last_watched_at))| SeriesEntry {
            anime_id,
            name,
            has_unwatched,
            last_watched_at,
        })
        .collect();

    // Sort: unwatched first → most recently watched → alphabetical
    entries.sort_by(|a, b| {
        b.has_unwatched
            .cmp(&a.has_unwatched)
            .then_with(|| b.last_watched_at.cmp(&a.last_watched_at))
            .then_with(|| a.name.cmp(&b.name))
    });

    entries
}

/// Find the filename of the next unwatched episode for a series.
/// Returns the filename (not path) of the first unwatched file sorted by episode number.
pub fn next_unwatched_filename(
    crdt: &CrdtState,
    storage: &ClientStorage,
    anime_id: u64,
) -> Option<String> {
    let watched: HashMap<FileId, u64> = storage
        .watched_files()
        .unwrap_or_default()
        .into_iter()
        .collect();

    // Collect unwatched files for this series with episode info
    let mut unwatched: Vec<(String, String)> = Vec::new(); // (episode_number, filename)

    for (file_id, (_ts, value)) in crdt.anidb.iter() {
        if let Some(meta) = value
            && meta.anime_id == anime_id
            && !watched.contains_key(file_id)
        {
            let has_local = storage
                .get_file_mapping(file_id)
                .ok()
                .flatten()
                .is_some();
            if has_local
                && let Some(filename) = crdt.filenames.read(file_id)
            {
                unwatched.push((meta.episode_number.clone(), filename.clone()));
            }
        }
    }

    // Sort by episode number (lexicographic is fine for "1", "2", ... and "S1", "C1", etc.)
    unwatched.sort_by(|a, b| a.0.cmp(&b.0));
    unwatched.into_iter().next().map(|(_, filename)| filename)
}

/// Find the directory containing files for a series.
/// Checks series_mapping_dirs first, then falls back to file_mapping directories.
pub fn series_directory(
    crdt: &CrdtState,
    storage: &ClientStorage,
    anime_id: u64,
) -> Option<std::path::PathBuf> {
    // 1. Check stored series mapping dir
    if let Ok(Some(dir)) = storage.get_series_mapping_dir(anime_id) {
        return Some(dir);
    }

    // 2. Fall back to directory of any mapped file from this series
    for (file_id, (_ts, value)) in crdt.anidb.iter() {
        if let Some(meta) = value
            && meta.anime_id == anime_id
            && let Ok(Some(path)) = storage.get_file_mapping(file_id)
            && let Some(dir) = path.parent()
        {
            return Some(dir.to_path_buf());
        }
    }

    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use dessplay_core::crdt::CrdtState;
    use dessplay_core::protocol::{CrdtOp, LwwValue};
    use dessplay_core::types::{AniDbMetadata, MetadataSource};

    fn fid(n: u8) -> FileId {
        let mut arr = [0u8; 16];
        arr[0] = n;
        FileId(arr)
    }

    fn make_meta(anime_id: u64, name: &str, episode: &str) -> AniDbMetadata {
        AniDbMetadata {
            anime_id,
            anime_name: name.to_string(),
            episode_number: episode.to_string(),
            episode_name: String::new(),
            group_name: String::new(),
            source: MetadataSource::AniDb,
        }
    }

    /// Create a temp dir with named media files and return (dir, file_paths).
    fn make_temp_files(names: &[&str]) -> (tempfile::TempDir, Vec<std::path::PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<_> = names
            .iter()
            .map(|name| {
                let p = dir.path().join(name);
                if let Some(parent) = p.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&p, b"test data").unwrap();
                p
            })
            .collect();
        (dir, paths)
    }

    #[test]
    fn empty_crdt_empty_list() {
        let crdt = CrdtState::new();
        let storage = ClientStorage::open_in_memory().unwrap();
        let list = build_series_list(&crdt, &storage);
        assert!(list.is_empty());
    }

    #[test]
    fn no_local_mapping_excluded() {
        let mut crdt = CrdtState::new();
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::AniDb(fid(1), Some(make_meta(42, "Frieren", "1"))),
        });
        let storage = ClientStorage::open_in_memory().unwrap();
        // No file mapping → not in list
        let list = build_series_list(&crdt, &storage);
        assert!(list.is_empty());
    }

    #[test]
    fn local_unwatched_file_shows_series() {
        let (_dir, paths) = make_temp_files(&["ep01.mkv"]);
        let mut crdt = CrdtState::new();
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::AniDb(fid(1), Some(make_meta(42, "Frieren", "1"))),
        });
        let storage = ClientStorage::open_in_memory().unwrap();
        storage.set_file_mapping(&fid(1), &paths[0]).unwrap();

        let list = build_series_list(&crdt, &storage);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].anime_id, 42);
        assert_eq!(list[0].name, "Frieren");
        assert!(list[0].has_unwatched);
        assert!(list[0].last_watched_at.is_none());
    }

    #[test]
    fn watched_file_no_unwatched_flag() {
        let (_dir, paths) = make_temp_files(&["ep01.mkv"]);
        let mut crdt = CrdtState::new();
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::AniDb(fid(1), Some(make_meta(42, "Frieren", "1"))),
        });
        let storage = ClientStorage::open_in_memory().unwrap();
        storage.set_file_mapping(&fid(1), &paths[0]).unwrap();
        storage.mark_watched(&fid(1), 5000).unwrap();

        let list = build_series_list(&crdt, &storage);
        assert_eq!(list.len(), 1);
        assert!(!list[0].has_unwatched);
        assert_eq!(list[0].last_watched_at, Some(5000));
    }

    #[test]
    fn mixed_watched_unwatched() {
        let (_dir, paths) = make_temp_files(&["ep01.mkv", "ep02.mkv"]);
        let mut crdt = CrdtState::new();
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::AniDb(fid(1), Some(make_meta(42, "Frieren", "1"))),
        });
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 101,
            value: LwwValue::AniDb(fid(2), Some(make_meta(42, "Frieren", "2"))),
        });
        let storage = ClientStorage::open_in_memory().unwrap();
        storage.set_file_mapping(&fid(1), &paths[0]).unwrap();
        storage.set_file_mapping(&fid(2), &paths[1]).unwrap();
        storage.mark_watched(&fid(1), 5000).unwrap();

        let list = build_series_list(&crdt, &storage);
        assert_eq!(list.len(), 1);
        assert!(list[0].has_unwatched); // ep02 unwatched
        assert_eq!(list[0].last_watched_at, Some(5000)); // ep01 watched at 5000
    }

    #[test]
    fn sort_unwatched_first() {
        let (_dir, paths) = make_temp_files(&["frieren/ep01.mkv", "aot/ep01.mkv"]);
        let mut crdt = CrdtState::new();
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::AniDb(fid(1), Some(make_meta(42, "Frieren", "1"))),
        });
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 101,
            value: LwwValue::AniDb(fid(2), Some(make_meta(99, "Attack on Titan", "1"))),
        });
        let storage = ClientStorage::open_in_memory().unwrap();
        storage.set_file_mapping(&fid(1), &paths[0]).unwrap();
        storage.set_file_mapping(&fid(2), &paths[1]).unwrap();
        // Watch Frieren but not AoT
        storage.mark_watched(&fid(1), 5000).unwrap();

        let list = build_series_list(&crdt, &storage);
        assert_eq!(list.len(), 2);
        // AoT first (has unwatched), Frieren second (all watched)
        assert_eq!(list[0].name, "Attack on Titan");
        assert!(list[0].has_unwatched);
        assert_eq!(list[1].name, "Frieren");
        assert!(!list[1].has_unwatched);
    }

    #[test]
    fn sort_recently_watched_before_alphabetical() {
        let (_dir, paths) = make_temp_files(&["a/ep01.mkv", "z/ep01.mkv"]);
        let mut crdt = CrdtState::new();
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::AniDb(fid(1), Some(make_meta(1, "Alpha", "1"))),
        });
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 101,
            value: LwwValue::AniDb(fid(2), Some(make_meta(2, "Zeta", "1"))),
        });
        let storage = ClientStorage::open_in_memory().unwrap();
        storage.set_file_mapping(&fid(1), &paths[0]).unwrap();
        storage.set_file_mapping(&fid(2), &paths[1]).unwrap();
        storage.mark_watched(&fid(1), 1000).unwrap();
        storage.mark_watched(&fid(2), 2000).unwrap();

        let list = build_series_list(&crdt, &storage);
        assert_eq!(list.len(), 2);
        // Zeta first (watched more recently), Alpha second
        assert_eq!(list[0].name, "Zeta");
        assert_eq!(list[1].name, "Alpha");
    }
}
