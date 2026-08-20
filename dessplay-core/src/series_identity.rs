//! Resolving a file to its Series Identity List entry (design.md, Series
//! Identity): AniDB linking is enrichment only, never a prerequisite for
//! commitment or gating -- every committable series routes through its
//! [`SeriesListEntry`](crate::types::SeriesListEntry) instead.

use std::collections::BTreeSet;

use digest::Digest;

use crate::state::StateView;
use crate::types::{AniDbSeriesId, Ed2kHash, ListEntryId, ListStatus, SeriesListEntry};

/// Resolve a file to the List entry that claims it, read-only. Resolution
/// order (design.md, Series Identity): an entry linked to the file's AniDB
/// series id, else an entry with the file's hash in `manual_files`, else
/// an entry whose `name`/`local_aliases` matches the file's derived series
/// name. `None` means nothing claims the file yet -- callers that need a
/// definite entry (committing via `/watch` etc.) auto-create one; this
/// function never does, since it also backs read-only gating derivation
/// (`derive::series_watch_for_file`), which must stay a pure query.
pub fn resolve_series_entry_for_file(view: &StateView, file: Ed2kHash) -> Option<ListEntryId> {
    // Steps 1 and 3 need metadata (a series id / a derived name); step 2
    // is a pure hash-membership test and must work without any — a
    // manually-attached file is committed the moment it's attached, not
    // once the server's fallback metadata lands.
    let metadata = view.anidb_metadata.get(&file).and_then(|m| m.as_ref());

    if let Some(series_id) = metadata.and_then(|m| m.series_id)
        && let Some((id, _)) = view
            .list_entries
            .iter()
            .find(|(_, entry)| entry.anidb_series_id == Some(series_id))
    {
        return Some(*id);
    }

    if let Some((id, _)) = view
        .list_entries
        .iter()
        .find(|(_, entry)| entry.manual_files.contains(&file))
    {
        return Some(*id);
    }

    let metadata = metadata?;
    view.list_entries
        .iter()
        .find(|(_, entry)| {
            entry.name == metadata.series_name
                || entry.local_aliases.contains(&metadata.series_name)
        })
        .map(|(id, _)| *id)
}

/// A prebuilt index over [`StateView::list_entries`] for resolving many
/// files in one pass: the same claims, the same order, as
/// [`resolve_series_entry_for_file`] (the `index_resolution_matches_the_scan`
/// proptest pins the equivalence), but a bulk caller pays O(entries) once
/// to build it instead of up to three linear entry scans per file. Built
/// per call from a view — it holds borrows and no freshness logic, so a
/// stale index is unrepresentable rather than merely avoided.
pub struct SeriesEntryIndex<'a> {
    /// First (lowest-id) entry linked to each AniDB series.
    by_series: std::collections::BTreeMap<AniDbSeriesId, ListEntryId>,
    /// First entry claiming each file via `manual_files`.
    by_manual: std::collections::BTreeMap<Ed2kHash, ListEntryId>,
    /// First entry claiming each name via `name` *or* `local_aliases`
    /// (within one entry both map to the same id, so folding them into
    /// one map preserves the scan's first-entry-wins order).
    by_name: std::collections::BTreeMap<&'a str, ListEntryId>,
}

impl<'a> SeriesEntryIndex<'a> {
    /// Index `view.list_entries` (one pass, ascending id order — the
    /// same order the scan visits, so every "first match wins" tie
    /// breaks identically).
    pub fn new(view: &'a StateView) -> Self {
        let mut by_series = std::collections::BTreeMap::new();
        let mut by_manual = std::collections::BTreeMap::new();
        let mut by_name = std::collections::BTreeMap::new();
        for (id, entry) in &view.list_entries {
            if let Some(series) = entry.anidb_series_id {
                by_series.entry(series).or_insert(*id);
            }
            for file in &entry.manual_files {
                by_manual.entry(*file).or_insert(*id);
            }
            by_name.entry(entry.name.as_str()).or_insert(*id);
            for alias in &entry.local_aliases {
                by_name.entry(alias.as_str()).or_insert(*id);
            }
        }
        Self {
            by_series,
            by_manual,
            by_name,
        }
    }

    /// [`resolve_series_entry_for_file`], through the index.
    pub fn resolve(&self, view: &StateView, file: Ed2kHash) -> Option<ListEntryId> {
        let metadata = view.anidb_metadata.get(&file).and_then(|m| m.as_ref());
        if let Some(series) = metadata.and_then(|m| m.series_id)
            && let Some(id) = self.by_series.get(&series)
        {
            return Some(*id);
        }
        if let Some(id) = self.by_manual.get(&file) {
            return Some(*id);
        }
        self.by_name.get(metadata?.series_name.as_str()).copied()
    }
}

/// Build a fresh List entry for a file that nothing claims yet, seeded
/// from its metadata -- linked (with `anidb_series_id`) when the file has
/// one, else unlinked with the derived name seeded as the sole
/// `local_aliases` entry so a later differently-hinted file for the same
/// show can still be matched by hand. `None` if the file has no metadata
/// at all (nothing to name an entry with).
pub fn build_entry_for_file(view: &StateView, file: Ed2kHash) -> Option<SeriesListEntry> {
    let metadata = view.anidb_metadata.get(&file)?.as_ref()?;
    Some(SeriesListEntry {
        name: metadata.series_name.clone(),
        nero_name: None,
        genre: None,
        notes: Vec::new(),
        recommender: None,
        status: ListStatus::Active,
        status_note: None,
        source: None,
        watchers: BTreeSet::new(),
        anidb_series_id: metadata.series_id,
        local_aliases: if metadata.series_id.is_none() {
            [metadata.series_name.clone()].into_iter().collect()
        } else {
            BTreeSet::new()
        },
        manual_files: BTreeSet::new(),
        anidb_unavailable: false,
    })
}

/// Deterministically derive a `ListEntryId` for a series identity: the
/// AniDB series id when linked, else the file's derived name. Used
/// whenever a `ListEntryId` is synthesized for a series that isn't
/// explicitly `/watch`-committed by a human (the migration's synthesis,
/// and auto-create on first commitment/gating) -- *not* for a real,
/// deliberate creation (`import.rs`'s CSV import), which still mints a
/// genuinely random id since there's no content to hash yet.
///
/// Determinism here is load-bearing, not just tidy: two clients racing to
/// auto-create an entry for the *same* series (e.g. two peers each
/// independently deciding "unknown series -> NotWatching" for a file
/// neither has watch history for) must converge on the same entry, or
/// gating/EOF-advance forks onto two different entries that never merge
/// back into one commitment (caught by an end-to-end test flaking on
/// exactly this race before this function existed).
pub fn derive_entry_id(anidb_series_id: Option<AniDbSeriesId>, derived_name: &str) -> ListEntryId {
    let mut hasher = md4::Md4::new();
    match anidb_series_id {
        Some(id) => {
            hasher.update(b"dessplay:list-entry:anidb:");
            hasher.update(id.0.to_le_bytes());
        }
        None => {
            hasher.update(b"dessplay:list-entry:name:");
            hasher.update(derived_name.as_bytes());
        }
    }
    ListEntryId::from_bytes(hasher.finalize().into())
}

/// Resolve `file` to its List entry, or the id and (unsaved) entry to
/// create if nothing claims it yet (design.md, Series Identity). `None`
/// only when the file has no metadata at all -- nothing to resolve or
/// name a fresh entry with. The synthesized id is deterministic (see
/// [`derive_entry_id`]): callers don't need `rand`, and independent
/// callers resolving the same series converge on the same entry.
pub fn resolve_or_build_entry(
    view: &StateView,
    file: Ed2kHash,
) -> Option<(ListEntryId, Option<SeriesListEntry>)> {
    if let Some(id) = resolve_series_entry_for_file(view, file) {
        return Some((id, None));
    }
    let metadata = view.anidb_metadata.get(&file)?.as_ref()?;
    let id = derive_entry_id(metadata.series_id, &metadata.series_name);
    let entry = build_entry_for_file(view, file)?;
    Some((id, Some(entry)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use proptest::prelude::*;

    use super::*;
    use crate::state::CrdtState;
    use crate::types::{ActorId, AniDbMetadata, MetadataSource, SharedTimestamp};

    const A: ActorId = ActorId::SERVER;

    fn hash(i: u8) -> Ed2kHash {
        Ed2kHash([i; 16])
    }

    fn ts(t: u64) -> SharedTimestamp {
        SharedTimestamp(t)
    }

    fn entry(name: &str) -> SeriesListEntry {
        SeriesListEntry {
            name: name.into(),
            nero_name: None,
            genre: None,
            notes: Vec::new(),
            recommender: None,
            status: ListStatus::Active,
            status_note: None,
            source: None,
            watchers: BTreeSet::new(),
            anidb_series_id: None,
            local_aliases: BTreeSet::new(),
            manual_files: BTreeSet::new(),
            anidb_unavailable: false,
        }
    }

    fn fallback_metadata(state: &mut CrdtState, file: Ed2kHash, series_name: &str) {
        state.set_anidb_metadata(
            A,
            ts(1),
            file,
            Some(AniDbMetadata {
                source: MetadataSource::FilenameDerived,
                series_name: series_name.into(),
                series_id: None,
                episode_number: None,
            }),
        );
    }

    /// Resolution step 2: `manual_files` is a pure hash-membership test
    /// and must resolve even before any metadata has been synced for the
    /// file (design.md, Series Identity — the step has no name or AniDB
    /// dependency). Regression: the metadata guard used to sit above it,
    /// so a manually-attached file resolved to nothing (Maybe gating)
    /// until the server's fallback metadata landed.
    #[test]
    fn manual_files_resolves_a_file_with_no_metadata_at_all() {
        let mut state = CrdtState::new();
        let mut e = entry("Some Obscure Show");
        e.manual_files.insert(hash(1));
        state.put_list_entry(A, ts(1), ListEntryId(7), e);
        assert_eq!(
            resolve_series_entry_for_file(&state.view(), hash(1)),
            Some(ListEntryId(7)),
        );
    }

    /// Step 3, `name` half: the derived name matching an entry's name
    /// resolves to it.
    #[test]
    fn derived_name_matching_an_entry_name_resolves() {
        let mut state = CrdtState::new();
        fallback_metadata(&mut state, hash(1), "Some Obscure Show");
        state.put_list_entry(A, ts(2), ListEntryId(9), entry("Some Obscure Show"));
        assert_eq!(
            resolve_series_entry_for_file(&state.view(), hash(1)),
            Some(ListEntryId(9)),
        );
    }

    /// Step 3, `local_aliases` half: a derived name found only in an
    /// entry's aliases resolves to it.
    #[test]
    fn derived_name_matching_a_local_alias_resolves() {
        let mut state = CrdtState::new();
        fallback_metadata(&mut state, hash(1), "ObscureShow S2");
        let mut e = entry("Some Obscure Show");
        e.local_aliases.insert("ObscureShow S2".into());
        state.put_list_entry(A, ts(2), ListEntryId(9), e);
        assert_eq!(
            resolve_series_entry_for_file(&state.view(), hash(1)),
            Some(ListEntryId(9)),
        );
    }

    /// The design's motivating case (plan.md Phase 19): two files of the
    /// same show with *different* directory-derived hints — one from a
    /// dedicated folder, one loose — both resolve to the one entry once
    /// both hints are in its `local_aliases`.
    #[test]
    fn two_differently_hinted_files_resolve_to_the_same_entry_via_aliases() {
        let mut state = CrdtState::new();
        fallback_metadata(&mut state, hash(1), "Obscure Show");
        fallback_metadata(&mut state, hash(2), "obscure_show_ep2_loose");
        let mut e = entry("Some Obscure Show");
        e.local_aliases.insert("Obscure Show".into());
        e.local_aliases.insert("obscure_show_ep2_loose".into());
        state.put_list_entry(A, ts(3), ListEntryId(9), e);
        let view = state.view();
        let first = resolve_series_entry_for_file(&view, hash(1));
        let second = resolve_series_entry_for_file(&view, hash(2));
        assert_eq!(first, Some(ListEntryId(9)));
        assert_eq!(first, second);
    }

    proptest! {
        /// The design's whole resolution contract in one generated test:
        /// linked entry > manual file > name match > deterministic
        /// auto-create, and persisting the auto-created entry makes a
        /// repeated resolution a read-only hit on the same id.
        #[test]
        fn resolution_order_and_autocreate_are_deterministic(
            file_byte in any::<u8>(),
            series_raw in any::<u32>(),
            name in "[A-Za-z0-9 ]{1,32}",
        ) {
            let file = hash(file_byte);
            let series = AniDbSeriesId(series_raw);
            let metadata = AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: name.clone(),
                series_id: Some(series),
                episode_number: Some("1".into()),
            };

            let mut by_name_and_hash = CrdtState::new();
            by_name_and_hash.set_anidb_metadata(A, ts(1), file, Some(metadata.clone()));
            by_name_and_hash.put_list_entry(A, ts(2), ListEntryId(1), entry(&name));
            let mut manual = entry("manual claim");
            manual.manual_files.insert(file);
            by_name_and_hash.put_list_entry(A, ts(3), ListEntryId(2), manual);
            prop_assert_eq!(
                resolve_series_entry_for_file(&by_name_and_hash.view(), file),
                Some(ListEntryId(2)),
                "manual_files must beat a name match",
            );

            let mut with_link = by_name_and_hash.clone();
            let mut linked = entry("linked claim");
            linked.anidb_series_id = Some(series);
            with_link.put_list_entry(A, ts(4), ListEntryId(3), linked);
            prop_assert_eq!(
                resolve_series_entry_for_file(&with_link.view(), file),
                Some(ListEntryId(3)),
                "an AniDB link must beat a manual_files claim",
            );

            let mut fresh = CrdtState::new();
            fresh.set_anidb_metadata(A, ts(1), file, Some(metadata));
            let (created_id, created) = resolve_or_build_entry(&fresh.view(), file)
                .expect("metadata always permits auto-creation");
            let repeated_id = resolve_or_build_entry(&fresh.view(), file)
                .expect("the same unresolved view remains resolvable")
                .0;
            prop_assert_eq!(created_id, repeated_id);
            fresh.put_list_entry(
                A,
                ts(2),
                created_id,
                created.expect("the first resolution must build an entry"),
            );
            prop_assert_eq!(
                resolve_or_build_entry(&fresh.view(), file),
                Some((created_id, None)),
            );

            let other_series = AniDbSeriesId(series_raw.wrapping_add(1));
            prop_assert_ne!(
                derive_entry_id(Some(series), &name),
                derive_entry_id(Some(other_series), &name),
            );
            prop_assert_ne!(
                derive_entry_id(Some(series), &name),
                derive_entry_id(None, &name),
                "linked and unlinked ids use separate hash domains",
            );
        }

        /// [`SeriesEntryIndex`] resolves exactly like the per-file scan
        /// for every file, over arbitrary entry mixtures. The tiny
        /// alphabets are deliberate: they force shared names, shared
        /// series links, and competing manual claims, so the scan's
        /// first-entry-wins tie-breaks are exercised, not dodged.
        #[test]
        fn index_resolution_matches_the_scan(
            entries in proptest::collection::vec(
                (
                    proptest::option::of(0u32..4),                       // anidb link
                    "[a-d]{1,2}",                                        // name
                    proptest::collection::btree_set("[a-d]{1,2}", 0..3), // aliases
                    proptest::collection::btree_set(0u8..6, 0..3),       // manual files
                ),
                0..6,
            ),
            files in proptest::collection::vec(
                (
                    0u8..6,                                              // hash
                    proptest::option::of((proptest::option::of(0u32..4), "[a-d]{1,2}")),
                ),
                0..8,
            ),
        ) {
            let mut state = CrdtState::new();
            for (i, (series, name, aliases, manual)) in entries.into_iter().enumerate() {
                let mut e = entry(&name);
                e.anidb_series_id = series.map(AniDbSeriesId);
                e.local_aliases = aliases;
                e.manual_files = manual.into_iter().map(hash).collect();
                state.put_list_entry(A, ts(i as u64 + 1), ListEntryId(i as u128), e);
            }
            for (i, (file, metadata)) in files.into_iter().enumerate() {
                state.set_anidb_metadata(
                    A,
                    ts(100 + i as u64),
                    hash(file),
                    metadata.map(|(series, name)| AniDbMetadata {
                        source: MetadataSource::FilenameDerived,
                        series_name: name,
                        series_id: series.map(AniDbSeriesId),
                        episode_number: None,
                    }),
                );
            }
            let view = state.view();
            let index = SeriesEntryIndex::new(&view);
            for file in (0u8..6).map(hash) {
                prop_assert_eq!(
                    index.resolve(&view, file),
                    resolve_series_entry_for_file(&view, file),
                    "index and scan disagree on {:?}", file,
                );
            }
        }
    }
}
