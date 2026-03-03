use std::collections::HashMap;

use dessplay_core::crdt::CrdtState;
use dessplay_core::types::FileId;

use crate::storage::ClientStorage;

// ---------------------------------------------------------------------------
// Franchise grouping constants
// ---------------------------------------------------------------------------

/// Relation types to INCLUDE when building franchise groups.
/// Excludes: 41 (music video), 100 (other).
const FRANCHISE_RELATION_TYPES: &[u16] = &[
    1,  // sequel
    2,  // prequel
    11, // same setting
    12, // alt setting
    32, // alt version
    42, // character
    51, // side story
    52, // parent story
    61, // summary
    62, // full story
];

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A member of a franchise (one anime_id).
#[derive(Clone, Debug)]
pub struct FranchiseMember {
    pub anime_id: u64,
    pub name: String,
    pub year: Option<u32>,
}

/// A franchise entry for the series pane.
/// Groups related anime_ids into one display entry.
#[derive(Clone, Debug)]
pub struct FranchiseEntry {
    /// Representative anime_id (smallest in group).
    pub franchise_id: u64,
    /// Display name (name of the member with the earliest year, or alpha-first).
    pub name: String,
    /// All members of this franchise, sorted by year then name.
    pub members: Vec<FranchiseMember>,
    /// Whether any member has unwatched local files.
    pub has_unwatched: bool,
    /// Max watched timestamp across all members.
    pub last_watched_at: Option<u64>,
    /// Earliest year across members (for sort-by-year).
    pub year: Option<u32>,
}

/// Backward-compatible alias used by display_data.rs.
pub type SeriesEntry = FranchiseEntry;

// ---------------------------------------------------------------------------
// Union-Find
// ---------------------------------------------------------------------------

struct UnionFind {
    parent: HashMap<u64, u64>,
    rank: HashMap<u64, u32>,
}

impl UnionFind {
    fn new() -> Self {
        Self {
            parent: HashMap::new(),
            rank: HashMap::new(),
        }
    }

    fn make_set(&mut self, x: u64) {
        self.parent.entry(x).or_insert(x);
        self.rank.entry(x).or_insert(0);
    }

    fn find(&mut self, x: u64) -> u64 {
        let p = *self.parent.get(&x).unwrap_or(&x);
        if p == x {
            return x;
        }
        let root = self.find(p);
        self.parent.insert(x, root);
        root
    }

    fn union(&mut self, x: u64, y: u64) {
        let rx = self.find(x);
        let ry = self.find(y);
        if rx == ry {
            return;
        }
        let rank_x = *self.rank.get(&rx).unwrap_or(&0);
        let rank_y = *self.rank.get(&ry).unwrap_or(&0);
        if rank_x < rank_y {
            self.parent.insert(rx, ry);
        } else if rank_x > rank_y {
            self.parent.insert(ry, rx);
        } else {
            self.parent.insert(ry, rx);
            *self.rank.entry(rx).or_insert(0) += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Building franchise list
// ---------------------------------------------------------------------------

/// Intermediate per-anime_id data gathered from the CRDT.
struct AnimeInfo {
    name: String,
    year: Option<u32>,
    related_aids: Vec<(u64, u16)>,
    has_unwatched: bool,
    last_watched_at: Option<u64>,
    has_local_file: bool,
}

/// Build the list of franchises for the series pane.
///
/// Groups related anime_ids by AniDB relations graph (filtered by allowed
/// relation types). Only includes franchises with at least one locally-mapped file.
/// Sort: unwatched first → most recently watched → alphabetical (same as old build_series_list).
pub fn build_series_list(crdt: &CrdtState, storage: &ClientStorage) -> Vec<FranchiseEntry> {
    build_franchise_list(crdt, storage)
}

/// Build franchise list with the "recent" sort (unwatched → recency → alpha).
pub fn build_franchise_list(crdt: &CrdtState, storage: &ClientStorage) -> Vec<FranchiseEntry> {
    let mut entries = build_franchise_list_unsorted(crdt, storage);
    sort_recent(&mut entries);
    entries
}

/// Build franchise list sorted by title (case-insensitive alphabetical).
pub fn build_franchise_list_by_title(
    crdt: &CrdtState,
    storage: &ClientStorage,
) -> Vec<FranchiseEntry> {
    let mut entries = build_franchise_list_unsorted(crdt, storage);
    sort_by_title(&mut entries);
    entries
}

/// Build franchise list sorted by year (descending, None at end).
pub fn build_franchise_list_by_year(
    crdt: &CrdtState,
    storage: &ClientStorage,
) -> Vec<FranchiseEntry> {
    let mut entries = build_franchise_list_unsorted(crdt, storage);
    sort_by_year(&mut entries);
    entries
}

fn build_franchise_list_unsorted(
    crdt: &CrdtState,
    storage: &ClientStorage,
) -> Vec<FranchiseEntry> {
    let watched: HashMap<FileId, u64> = storage
        .watched_files()
        .unwrap_or_default()
        .into_iter()
        .collect();

    // 1. Collect per-anime_id info
    let mut anime_map: HashMap<u64, AnimeInfo> = HashMap::new();

    for (file_id, (_ts, value)) in crdt.anidb.iter() {
        if let Some(meta) = value {
            let has_local = storage
                .get_file_mapping(file_id)
                .ok()
                .flatten()
                .is_some();

            let entry = anime_map.entry(meta.anime_id).or_insert_with(|| AnimeInfo {
                name: meta.anime_name.clone(),
                year: meta.year,
                related_aids: meta.related_aids.clone(),
                has_unwatched: false,
                last_watched_at: None,
                has_local_file: false,
            });

            if has_local {
                entry.has_local_file = true;
                if let Some(ts) = watched.get(file_id) {
                    entry.last_watched_at =
                        Some(entry.last_watched_at.map_or(*ts, |prev| prev.max(*ts)));
                } else {
                    entry.has_unwatched = true;
                }
            }
        }
    }

    // 2. Build union-find from related_aids (filtered by allowed types)
    let mut uf = UnionFind::new();
    for &aid in anime_map.keys() {
        uf.make_set(aid);
    }
    for (aid, info) in &anime_map {
        for &(related_aid, relation_type) in &info.related_aids {
            if FRANCHISE_RELATION_TYPES.contains(&relation_type) && anime_map.contains_key(&related_aid) {
                uf.union(*aid, related_aid);
            }
        }
    }

    // 3. Group by connected component
    let mut groups: HashMap<u64, Vec<u64>> = HashMap::new();
    for &aid in anime_map.keys() {
        let root = uf.find(aid);
        groups.entry(root).or_default().push(aid);
    }

    // 4. Build FranchiseEntry for each group
    let mut entries = Vec::new();
    for (_root, mut member_aids) in groups {
        // Only include franchises with at least one locally-mapped file
        let any_local = member_aids
            .iter()
            .any(|aid| anime_map[aid].has_local_file);
        if !any_local {
            continue;
        }

        member_aids.sort();

        let franchise_id = member_aids[0];

        // Compute aggregate stats
        let has_unwatched = member_aids
            .iter()
            .any(|aid| anime_map[aid].has_unwatched);
        let last_watched_at = member_aids
            .iter()
            .filter_map(|aid| anime_map[aid].last_watched_at)
            .max();
        let earliest_year = member_aids
            .iter()
            .filter_map(|aid| anime_map[aid].year)
            .min();

        // Build members sorted by year then name
        let mut members: Vec<FranchiseMember> = member_aids
            .iter()
            .map(|aid| {
                let info = &anime_map[aid];
                FranchiseMember {
                    anime_id: *aid,
                    name: info.name.clone(),
                    year: info.year,
                }
            })
            .collect();
        members.sort_by(|a, b| {
            a.year
                .cmp(&b.year)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        // Franchise name: name of the member with the earliest year,
        // or alphabetically first if no years.
        let name = members
            .iter()
            .min_by(|a, b| {
                match (a.year, b.year) {
                    (Some(ya), Some(yb)) => ya.cmp(&yb),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            })
            .map(|m| m.name.clone())
            .unwrap_or_default();

        entries.push(FranchiseEntry {
            franchise_id,
            name,
            members,
            has_unwatched,
            last_watched_at,
            year: earliest_year,
        });
    }

    entries
}

// ---------------------------------------------------------------------------
// Sort functions
// ---------------------------------------------------------------------------

/// Sort: unwatched first → most recently watched → alphabetical.
pub fn sort_recent(entries: &mut [FranchiseEntry]) {
    entries.sort_by(|a, b| {
        b.has_unwatched
            .cmp(&a.has_unwatched)
            .then_with(|| b.last_watched_at.cmp(&a.last_watched_at))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

/// Sort: case-insensitive alphabetical by name.
pub fn sort_by_title(entries: &mut [FranchiseEntry]) {
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
}

/// Sort: year descending → alpha within year. None-year at end.
pub fn sort_by_year(entries: &mut [FranchiseEntry]) {
    entries.sort_by(|a, b| {
        match (a.year, b.year) {
            (Some(ya), Some(yb)) => yb.cmp(&ya), // descending
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

// ---------------------------------------------------------------------------
// Per-franchise helpers
// ---------------------------------------------------------------------------

/// Find the filename of the next unwatched episode for a franchise.
/// Searches across all anime_ids in the franchise.
pub fn next_unwatched_filename(
    crdt: &CrdtState,
    storage: &ClientStorage,
    anime_id: u64,
) -> Option<String> {
    next_unwatched_filename_for_ids(crdt, storage, &[anime_id])
}

/// Find the next unwatched filename across multiple anime_ids.
pub fn next_unwatched_filename_for_ids(
    crdt: &CrdtState,
    storage: &ClientStorage,
    anime_ids: &[u64],
) -> Option<String> {
    let watched: HashMap<FileId, u64> = storage
        .watched_files()
        .unwrap_or_default()
        .into_iter()
        .collect();

    let mut unwatched: Vec<(String, String)> = Vec::new();

    for (file_id, (_ts, value)) in crdt.anidb.iter() {
        if let Some(meta) = value
            && anime_ids.contains(&meta.anime_id)
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

    unwatched.sort_by(|a, b| a.0.cmp(&b.0));
    unwatched.into_iter().next().map(|(_, filename)| filename)
}

/// Find the directory containing files for a franchise.
/// Checks series_mapping_dirs for all anime_ids, then falls back to file_mapping directories.
pub fn series_directory(
    crdt: &CrdtState,
    storage: &ClientStorage,
    anime_id: u64,
) -> Option<std::path::PathBuf> {
    series_directory_for_ids(crdt, storage, &[anime_id])
}

/// Find the directory for any of the given anime_ids.
pub fn series_directory_for_ids(
    crdt: &CrdtState,
    storage: &ClientStorage,
    anime_ids: &[u64],
) -> Option<std::path::PathBuf> {
    // 1. Check stored series mapping dirs
    for &aid in anime_ids {
        if let Ok(Some(dir)) = storage.get_series_mapping_dir(aid) {
            return Some(dir);
        }
    }

    // 2. Fall back to directory of any mapped file from any of these series
    for (file_id, (_ts, value)) in crdt.anidb.iter() {
        if let Some(meta) = value
            && anime_ids.contains(&meta.anime_id)
            && let Ok(Some(path)) = storage.get_file_mapping(file_id)
            && let Some(dir) = path.parent()
        {
            return Some(dir.to_path_buf());
        }
    }

    None
}

/// Collect episodes for a given anime_id from the CRDT.
/// Returns a list sorted by episode number, suitable for the episode browser.
pub fn episodes_for_anime_id(
    crdt: &CrdtState,
    storage: &ClientStorage,
    anime_id: u64,
) -> Vec<crate::tui::ui_state::EpisodeEntry> {
    let mut episodes = Vec::new();
    for (file_id, (_ts, value)) in crdt.anidb.iter() {
        if let Some(meta) = value
            && meta.anime_id == anime_id
        {
            let has_local = storage
                .get_file_mapping(file_id)
                .ok()
                .flatten()
                .is_some();
            episodes.push(crate::tui::ui_state::EpisodeEntry {
                file_id: *file_id,
                number: meta.episode_number.clone(),
                name: meta.episode_name.clone(),
                has_local,
            });
        }
    }
    // Sort by episode number: try numeric sort, fall back to lexicographic
    episodes.sort_by(|a, b| {
        let na = a.number.parse::<f64>();
        let nb = b.number.parse::<f64>();
        match (na, nb) {
            (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
            _ => a.number.cmp(&b.number),
        }
    });
    episodes
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
            year: None,
            related_aids: Vec::new(),
        }
    }

    fn make_meta_with_year(
        anime_id: u64,
        name: &str,
        episode: &str,
        year: Option<u32>,
        related_aids: Vec<(u64, u16)>,
    ) -> AniDbMetadata {
        AniDbMetadata {
            anime_id,
            anime_name: name.to_string(),
            episode_number: episode.to_string(),
            episode_name: String::new(),
            group_name: String::new(),
            source: MetadataSource::AniDb,
            year,
            related_aids,
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

    // --- Existing tests (backward compat) ---

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
        assert!(list[0].has_unwatched);
        assert_eq!(list[0].last_watched_at, Some(5000));
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
        storage.mark_watched(&fid(1), 5000).unwrap();

        let list = build_series_list(&crdt, &storage);
        assert_eq!(list.len(), 2);
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
        assert_eq!(list[0].name, "Zeta");
        assert_eq!(list[1].name, "Alpha");
    }

    // --- Franchise grouping tests ---

    #[test]
    fn franchise_groups_related_anime_ids() {
        // A→B (sequel), B→C (sequel) → one franchise {A, B, C}
        let (_dir, paths) = make_temp_files(&["a.mkv", "b.mkv", "c.mkv"]);
        let mut crdt = CrdtState::new();
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::AniDb(
                fid(1),
                Some(make_meta_with_year(10, "Series A", "1", Some(2020), vec![(20, 1)])),
            ),
        });
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 101,
            value: LwwValue::AniDb(
                fid(2),
                Some(make_meta_with_year(
                    20,
                    "Series B",
                    "1",
                    Some(2021),
                    vec![(10, 2), (30, 1)],
                )),
            ),
        });
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 102,
            value: LwwValue::AniDb(
                fid(3),
                Some(make_meta_with_year(30, "Series C", "1", Some(2022), vec![(20, 2)])),
            ),
        });
        let storage = ClientStorage::open_in_memory().unwrap();
        storage.set_file_mapping(&fid(1), &paths[0]).unwrap();
        storage.set_file_mapping(&fid(2), &paths[1]).unwrap();
        storage.set_file_mapping(&fid(3), &paths[2]).unwrap();

        let list = build_franchise_list(&crdt, &storage);
        assert_eq!(list.len(), 1, "Three related anime should form one franchise");
        assert_eq!(list[0].members.len(), 3);
        // Name should be from earliest year (Series A, 2020)
        assert_eq!(list[0].name, "Series A");
        assert_eq!(list[0].year, Some(2020));
    }

    #[test]
    fn franchise_excludes_music_video_relations() {
        // A→B (music video, type 41) → two separate franchises
        let (_dir, paths) = make_temp_files(&["a.mkv", "b.mkv"]);
        let mut crdt = CrdtState::new();
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::AniDb(
                fid(1),
                Some(make_meta_with_year(10, "Main Series", "1", Some(2020), vec![(20, 41)])),
            ),
        });
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 101,
            value: LwwValue::AniDb(
                fid(2),
                Some(make_meta_with_year(20, "Music Video", "1", Some(2020), vec![(10, 41)])),
            ),
        });
        let storage = ClientStorage::open_in_memory().unwrap();
        storage.set_file_mapping(&fid(1), &paths[0]).unwrap();
        storage.set_file_mapping(&fid(2), &paths[1]).unwrap();

        let list = build_franchise_list(&crdt, &storage);
        assert_eq!(list.len(), 2, "Music video relation should not merge franchises");
    }

    #[test]
    fn franchise_disconnected_components() {
        // A→B and C→D produce two separate groups
        let (_dir, paths) = make_temp_files(&["a.mkv", "b.mkv", "c.mkv", "d.mkv"]);
        let mut crdt = CrdtState::new();
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::AniDb(
                fid(1),
                Some(make_meta_with_year(10, "Group1A", "1", Some(2020), vec![(20, 1)])),
            ),
        });
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 101,
            value: LwwValue::AniDb(
                fid(2),
                Some(make_meta_with_year(20, "Group1B", "1", Some(2021), vec![(10, 2)])),
            ),
        });
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 102,
            value: LwwValue::AniDb(
                fid(3),
                Some(make_meta_with_year(30, "Group2A", "1", Some(2019), vec![(40, 1)])),
            ),
        });
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 103,
            value: LwwValue::AniDb(
                fid(4),
                Some(make_meta_with_year(40, "Group2B", "1", Some(2020), vec![(30, 2)])),
            ),
        });
        let storage = ClientStorage::open_in_memory().unwrap();
        for (i, p) in paths.iter().enumerate() {
            storage
                .set_file_mapping(&fid((i + 1) as u8), p)
                .unwrap();
        }

        let list = build_franchise_list(&crdt, &storage);
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn franchise_single_anime_no_relations() {
        let (_dir, paths) = make_temp_files(&["ep.mkv"]);
        let mut crdt = CrdtState::new();
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::AniDb(fid(1), Some(make_meta(42, "Standalone", "1"))),
        });
        let storage = ClientStorage::open_in_memory().unwrap();
        storage.set_file_mapping(&fid(1), &paths[0]).unwrap();

        let list = build_franchise_list(&crdt, &storage);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].members.len(), 1);
        assert_eq!(list[0].name, "Standalone");
    }

    #[test]
    fn franchise_name_uses_earliest_year() {
        let (_dir, paths) = make_temp_files(&["a.mkv", "b.mkv"]);
        let mut crdt = CrdtState::new();
        // B has earlier year (2018), A has 2020
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::AniDb(
                fid(1),
                Some(make_meta_with_year(10, "Sequel", "1", Some(2020), vec![(20, 2)])),
            ),
        });
        crdt.apply_op(&CrdtOp::LwwWrite {
            timestamp: 101,
            value: LwwValue::AniDb(
                fid(2),
                Some(make_meta_with_year(20, "Original", "1", Some(2018), vec![(10, 1)])),
            ),
        });
        let storage = ClientStorage::open_in_memory().unwrap();
        storage.set_file_mapping(&fid(1), &paths[0]).unwrap();
        storage.set_file_mapping(&fid(2), &paths[1]).unwrap();

        let list = build_franchise_list(&crdt, &storage);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Original");
    }

    // --- Sort function tests ---

    #[test]
    fn sort_by_title_case_insensitive() {
        let mut entries = vec![
            FranchiseEntry {
                franchise_id: 1,
                name: "Zeta".into(),
                members: Vec::new(),
                has_unwatched: false,
                last_watched_at: None,
                year: None,
            },
            FranchiseEntry {
                franchise_id: 2,
                name: "alpha".into(),
                members: Vec::new(),
                has_unwatched: false,
                last_watched_at: None,
                year: None,
            },
            FranchiseEntry {
                franchise_id: 3,
                name: "Beta".into(),
                members: Vec::new(),
                has_unwatched: false,
                last_watched_at: None,
                year: None,
            },
        ];
        sort_by_title(&mut entries);
        assert_eq!(entries[0].name, "alpha");
        assert_eq!(entries[1].name, "Beta");
        assert_eq!(entries[2].name, "Zeta");
    }

    #[test]
    fn sort_by_year_descending_none_last() {
        let mut entries = vec![
            FranchiseEntry {
                franchise_id: 1,
                name: "Old".into(),
                members: Vec::new(),
                has_unwatched: false,
                last_watched_at: None,
                year: Some(2018),
            },
            FranchiseEntry {
                franchise_id: 2,
                name: "Unknown".into(),
                members: Vec::new(),
                has_unwatched: false,
                last_watched_at: None,
                year: None,
            },
            FranchiseEntry {
                franchise_id: 3,
                name: "New".into(),
                members: Vec::new(),
                has_unwatched: false,
                last_watched_at: None,
                year: Some(2023),
            },
        ];
        sort_by_year(&mut entries);
        assert_eq!(entries[0].name, "New"); // 2023 first (descending)
        assert_eq!(entries[1].name, "Old"); // 2018
        assert_eq!(entries[2].name, "Unknown"); // None last
    }

    // --- Union-find tests ---

    #[test]
    fn union_find_basic() {
        let mut uf = UnionFind::new();
        uf.make_set(1);
        uf.make_set(2);
        uf.make_set(3);
        uf.union(1, 2);
        uf.union(2, 3);
        assert_eq!(uf.find(1), uf.find(3));
    }

    #[test]
    fn union_find_disjoint() {
        let mut uf = UnionFind::new();
        uf.make_set(1);
        uf.make_set(2);
        uf.make_set(3);
        uf.make_set(4);
        uf.union(1, 2);
        uf.union(3, 4);
        assert_eq!(uf.find(1), uf.find(2));
        assert_eq!(uf.find(3), uf.find(4));
        assert_ne!(uf.find(1), uf.find(3));
    }
}
