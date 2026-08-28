//! Resolving a file to its Series Identity List entry (design.md, Series
//! Identity): AniDB linking is enrichment only, never a prerequisite for
//! commitment or gating -- every committable series routes through its
//! [`SeriesListEntry`](crate::types::SeriesListEntry) instead.

use std::collections::BTreeSet;

use digest::Digest;

use crate::state::StateView;
use crate::types::{AniDbSeriesId, Ed2kHash, ListEntryId, ListStatus, SeriesListEntry};

/// Resolve a file to the List entry that claims it, read-only. Resolution
/// order (design.md, Series Identity): an entry linked into the file's
/// AniDB *franchise* (any season — see [`entries_claiming_series`]), else
/// an entry with the file's hash in `manual_files`, else an entry whose
/// `name`/`local_aliases` matches the file's derived series name. `None`
/// means nothing claims the file yet -- callers that need a definite entry
/// (committing via `/watch` etc.) auto-create one; this function never
/// does, since it also backs read-only gating derivation
/// (`derive::series_watch_for_file`), which must stay a pure query.
///
/// One implementation: this builds a [`SeriesEntryIndex`] (O(entries), the
/// same cost as the old three-scan resolution) and asks it, so the bulk
/// and single-file paths cannot drift apart.
pub fn resolve_series_entry_for_file(view: &StateView, file: Ed2kHash) -> Option<ListEntryId> {
    SeriesEntryIndex::new(view).resolve(view, file)
}

/// Every List entry linked into `series`' franchise, canonical first
/// (proposal 2026-08-28, "The List at franchise granularity"). A linked
/// entry claims `series` when its own season is in the component reachable
/// from `series` ([`franchise::reachable_component`]), *or* its season's
/// relations row names `series` directly — the one-hop backstop for a
/// brand-new season whose own relations row hasn't landed yet, which is
/// exactly the window in which a duplicate entry would otherwise be
/// auto-created. Empty when nothing claims the franchise.
///
/// Canonical order (only matters for legacy per-season duplicates): a
/// human-created entry (one whose id is not the auto-create hash of its
/// link, [`derive_entry_id`]) beats an auto-created one — a fresh season
/// entry auto-created in that window must never hide the entry carrying
/// the notes; then the entry linked deepest along the prequel chain (it
/// holds the live `next_ep`); then the lowest id.
pub fn entries_claiming_series(view: &StateView, series: AniDbSeriesId) -> Vec<ListEntryId> {
    SeriesEntryIndex::new(view).claimants(view, series)
}

/// Order List entries canonical-first (see [`entries_claiming_series`]):
/// human-created before auto-created, then deepest along the prequel
/// chain, then lowest id. Dedups. The one definition of "which of these
/// entries speaks for the franchise", shared by file resolution and The
/// List's one-row-per-franchise rendering.
pub fn canonical_first(view: &StateView, ids: &mut Vec<ListEntryId>) {
    ids.sort_unstable();
    ids.dedup();
    // `sort_by_key` is stable over the ascending-id order.
    ids.sort_by_key(|id| {
        let link = view
            .list_entries
            .get(id)
            .and_then(|entry| entry.anidb_series_id);
        let auto_created = link.is_some_and(|series| derive_entry_id(Some(series), "") == *id);
        let ordinal = link.map_or(0, |series| crate::franchise::season_ordinal(view, series));
        (auto_created, std::cmp::Reverse(ordinal))
    });
}

/// A prebuilt index over [`StateView::list_entries`] for resolving many
/// files in one pass: a bulk caller pays O(entries) once to build it
/// instead of up to three linear entry scans per file. Built per call
/// from a view — it holds borrows and no freshness logic, so a stale index
/// is unrepresentable rather than merely avoided.
pub struct SeriesEntryIndex<'a> {
    /// Entries linked to each AniDB series, ascending id.
    by_series: std::collections::BTreeMap<AniDbSeriesId, Vec<ListEntryId>>,
    /// Series named by a structural edge *from* a linked entry's season ->
    /// those entries (the one-hop backstop, see [`entries_claiming_series`]).
    by_neighbour: std::collections::BTreeMap<AniDbSeriesId, Vec<ListEntryId>>,
    /// First entry claiming each file via `manual_files`.
    by_manual: std::collections::BTreeMap<Ed2kHash, ListEntryId>,
    /// First entry claiming each name via `name` *or* `local_aliases`
    /// (within one entry both map to the same id, so folding them into
    /// one map preserves the scan's first-entry-wins order).
    by_name: std::collections::BTreeMap<&'a str, ListEntryId>,
}

impl<'a> SeriesEntryIndex<'a> {
    /// Index `view.list_entries` (one pass, ascending id order, so every
    /// "first match wins" tie breaks identically).
    pub fn new(view: &'a StateView) -> Self {
        let mut by_series: std::collections::BTreeMap<AniDbSeriesId, Vec<ListEntryId>> =
            std::collections::BTreeMap::new();
        let mut by_neighbour: std::collections::BTreeMap<AniDbSeriesId, Vec<ListEntryId>> =
            std::collections::BTreeMap::new();
        let mut by_manual = std::collections::BTreeMap::new();
        let mut by_name = std::collections::BTreeMap::new();
        for (id, entry) in &view.list_entries {
            if let Some(series) = entry.anidb_series_id {
                by_series.entry(series).or_default().push(*id);
                if let Some(relations) = view.series_relations.get(&series) {
                    for relation in &relations.relations {
                        if relation.kind.groups_franchise() {
                            by_neighbour.entry(relation.target).or_default().push(*id);
                        }
                    }
                }
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
            by_neighbour,
            by_manual,
            by_name,
        }
    }

    /// [`entries_claiming_series`], through the index.
    pub fn claimants(&self, view: &StateView, series: AniDbSeriesId) -> Vec<ListEntryId> {
        let mut ids: Vec<ListEntryId> = crate::franchise::reachable_component(view, series)
            .iter()
            .filter_map(|member| self.by_series.get(member))
            .flatten()
            .chain(self.by_neighbour.get(&series).into_iter().flatten())
            .copied()
            .collect();
        canonical_first(view, &mut ids);
        ids
    }

    /// [`resolve_series_entry_for_file`], through the index.
    pub fn resolve(&self, view: &StateView, file: Ed2kHash) -> Option<ListEntryId> {
        let metadata = view.anidb_metadata.get(&file).and_then(|m| m.as_ref());
        if let Some(series) = metadata.and_then(|m| m.series_id)
            && let Some(id) = self.claimants(view, series).first()
        {
            return Some(*id);
        }
        // Step 2 is a pure hash-membership test and must work without any
        // metadata — a manually-attached file is committed the moment it's
        // attached, not once the server's fallback metadata lands.
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
    use crate::types::{
        ActorId, AniDbMetadata, MetadataSource, RelationKind, SeriesRelation, SeriesRelations,
        SharedTimestamp,
    };

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

    /// A relations row: `title`, with structural edges to `targets`.
    fn relations(title: &str, targets: &[(RelationKind, u32)]) -> SeriesRelations {
        SeriesRelations {
            title: title.into(),
            year: None,
            episode_count: Some(12),
            relations: targets
                .iter()
                .map(|(kind, id)| SeriesRelation {
                    kind: *kind,
                    target: AniDbSeriesId(*id),
                })
                .collect(),
            short_titles: vec![],
        }
    }

    fn linked_metadata(state: &mut CrdtState, file: Ed2kHash, series: u32) {
        state.set_anidb_metadata(
            A,
            ts(1),
            file,
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: format!("series {series}"),
                series_id: Some(AniDbSeriesId(series)),
                episode_number: Some("1".into()),
            }),
        );
    }

    fn linked_entry(name: &str, series: u32) -> SeriesListEntry {
        let mut e = entry(name);
        e.anidb_series_id = Some(AniDbSeriesId(series));
        e
    }

    /// The franchise rule (proposal 2026-08-28): a file from a *sequel*
    /// season resolves to the entry linked to the prequel — one entry per
    /// franchise, so `/watch` on season three commits to the show, and
    /// `resolve_or_build_entry` must not mint a second, per-season entry.
    #[test]
    fn sequel_file_resolves_to_the_entry_linked_to_its_prequel() {
        let mut state = CrdtState::new();
        state.set_series_relations(
            A,
            ts(1),
            AniDbSeriesId(1),
            relations("S1", &[(RelationKind::Sequel, 2)]),
        );
        state.set_series_relations(
            A,
            ts(1),
            AniDbSeriesId(2),
            relations("S2", &[(RelationKind::Prequel, 1)]),
        );
        linked_metadata(&mut state, hash(2), 2);
        state.put_list_entry(A, ts(2), ListEntryId(7), linked_entry("Show", 1));
        let view = state.view();
        assert_eq!(
            resolve_series_entry_for_file(&view, hash(2)),
            Some(ListEntryId(7))
        );
        assert_eq!(
            resolve_or_build_entry(&view, hash(2)),
            Some((ListEntryId(7), None))
        );
    }

    /// The new-season window: the file's own season has no relations row
    /// yet (the ANIME lookup is still queued), but the linked season's row
    /// already names it as a Sequel. It must still resolve — this is
    /// exactly when a duplicate entry would otherwise be auto-created.
    #[test]
    fn unfetched_sequel_resolves_through_the_linked_seasons_edge() {
        let mut state = CrdtState::new();
        state.set_series_relations(
            A,
            ts(1),
            AniDbSeriesId(1),
            relations("S1", &[(RelationKind::Sequel, 2)]),
        );
        linked_metadata(&mut state, hash(2), 2);
        state.put_list_entry(A, ts(2), ListEntryId(7), linked_entry("Show", 1));
        assert_eq!(
            resolve_series_entry_for_file(&state.view(), hash(2)),
            Some(ListEntryId(7))
        );
    }

    /// Crossover edges don't group (Isekai Quartet is not Overlord): a file
    /// from a series reachable only through a non-structural edge stays
    /// unclaimed.
    #[test]
    fn a_crossover_neighbour_does_not_claim_the_file() {
        let mut state = CrdtState::new();
        state.set_series_relations(
            A,
            ts(1),
            AniDbSeriesId(1),
            relations("A", &[(RelationKind::Character, 2)]),
        );
        state.set_series_relations(
            A,
            ts(1),
            AniDbSeriesId(2),
            relations("B", &[(RelationKind::Character, 1)]),
        );
        linked_metadata(&mut state, hash(2), 2);
        state.put_list_entry(A, ts(2), ListEntryId(7), linked_entry("A", 1));
        assert_eq!(resolve_series_entry_for_file(&state.view(), hash(2)), None);
    }

    /// Legacy duplicates (several entries linked into one franchise): the
    /// canonical entry is a human-created one over an auto-created one,
    /// then the one linked deepest along the prequel chain (it holds the
    /// live `next_ep`), then the lowest id — and every season's file
    /// resolves to that same entry.
    #[test]
    fn canonical_entry_prefers_human_created_then_deepest_season() {
        let mut state = CrdtState::new();
        state.set_series_relations(
            A,
            ts(1),
            AniDbSeriesId(1),
            relations("S1", &[(RelationKind::Sequel, 2)]),
        );
        state.set_series_relations(
            A,
            ts(1),
            AniDbSeriesId(2),
            relations(
                "S2",
                &[(RelationKind::Prequel, 1), (RelationKind::Sequel, 3)],
            ),
        );
        state.set_series_relations(
            A,
            ts(1),
            AniDbSeriesId(3),
            relations("S3", &[(RelationKind::Prequel, 2)]),
        );
        for series in 1..=3 {
            linked_metadata(&mut state, hash(series as u8), series);
        }
        let auto = |series: u32| derive_entry_id(Some(AniDbSeriesId(series)), "");
        state.put_list_entry(A, ts(2), auto(1), linked_entry("S1", 1));
        state.put_list_entry(A, ts(2), auto(2), linked_entry("S2", 2));
        let view = state.view();
        for file in 1..=3u8 {
            assert_eq!(
                resolve_series_entry_for_file(&view, hash(file)),
                Some(auto(2)),
                "file {file}"
            );
        }
        assert_eq!(
            entries_claiming_series(&view, AniDbSeriesId(3)),
            vec![auto(2), auto(1)]
        );

        // A human-created entry (random id) linked to the *first* season
        // still wins over the auto-created deeper one.
        state.put_list_entry(A, ts(3), ListEntryId(99), linked_entry("Show", 1));
        let view = state.view();
        for file in 1..=3u8 {
            assert_eq!(
                resolve_series_entry_for_file(&view, hash(file)),
                Some(ListEntryId(99)),
                "file {file}"
            );
        }
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

        /// The franchise invariant: over a fully-fetched season chain
        /// (symmetric Sequel/Prequel rows) with any mixture of linked
        /// entries, *every* season's file resolves to the *same* entry
        /// (design.md, Series Identity: commitment is per franchise), that
        /// entry is the canonical claimant, and a human-created entry is
        /// canonical whenever one exists. Also pins the bulk index against
        /// the single-file path.
        #[test]
        fn every_season_of_a_franchise_resolves_to_one_canonical_entry(
            seasons in 1usize..6,
            // Per season: no entry / auto-created entry / human entry.
            links in proptest::collection::vec(0u8..3, 6),
        ) {
            let mut state = CrdtState::new();
            for season in 1..=seasons {
                let mut edges = Vec::new();
                if season > 1 {
                    edges.push((RelationKind::Prequel, season as u32 - 1));
                }
                if season < seasons {
                    edges.push((RelationKind::Sequel, season as u32 + 1));
                }
                state.set_series_relations(
                    A,
                    ts(1),
                    AniDbSeriesId(season as u32),
                    relations(&format!("S{season}"), &edges),
                );
                linked_metadata(&mut state, hash(season as u8), season as u32);
                let id = match links[season - 1] {
                    0 => continue,
                    1 => derive_entry_id(Some(AniDbSeriesId(season as u32)), ""),
                    _ => ListEntryId(1000 + season as u128),
                };
                state.put_list_entry(A, ts(2), id, linked_entry(&format!("S{season}"), season as u32));
            }
            let view = state.view();
            let index = SeriesEntryIndex::new(&view);
            let any_linked = links[..seasons].iter().any(|l| *l != 0);
            let any_human = links[..seasons].contains(&2);
            let resolved: BTreeSet<Option<ListEntryId>> = (1..=seasons)
                .map(|season| resolve_series_entry_for_file(&view, hash(season as u8)))
                .collect();
            prop_assert_eq!(resolved.len(), 1, "every season must resolve alike: {:?}", resolved);
            let resolved = resolved.into_iter().next().unwrap();
            prop_assert_eq!(resolved.is_some(), any_linked);
            for season in 1..=seasons {
                let file = hash(season as u8);
                prop_assert_eq!(index.resolve(&view, file), resolved);
                let claimants = entries_claiming_series(&view, AniDbSeriesId(season as u32));
                prop_assert_eq!(claimants.first().copied(), resolved);
                prop_assert_eq!(
                    claimants.len(),
                    links[..seasons].iter().filter(|l| **l != 0).count(),
                );
            }
            if let Some(id) = resolved {
                let human = view.list_entries[&id].anidb_series_id
                    .is_some_and(|s| derive_entry_id(Some(s), "") != id);
                prop_assert_eq!(human, any_human, "a human-created entry must be canonical");
            }
        }
    }
}
