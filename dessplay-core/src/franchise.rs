//! Franchise grouping: connected components over AniDB's relations
//! graph (sequel, prequel, side story...), with a parsed-name fallback
//! for files AniDB doesn't know.
//!
//! Clients build these from the server-authoritative `series_relations`
//! map (design.md, Parsing files to series). The graph fills in slowly
//! (Phase 8's rate-limited lookups), so groupings must degrade
//! gracefully: a series with no relations entry is its own franchise,
//! and a file with no series id groups by its metadata series name.

use std::collections::{BTreeMap, BTreeSet};

use crate::state::StateView;
use crate::types::{AniDbSeriesId, Ed2kHash};

/// What identifies a franchise: its AniDB component root, or a parsed
/// name for AniDB-unknown files.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum FranchiseKey {
    /// The smallest series id in the connected component.
    Series(AniDbSeriesId),
    /// Fallback: the metadata series name (filename-derived).
    Name(String),
}

/// One franchise: a group of related series and the known files that
/// belong to them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Franchise {
    /// Stable identity for selection.
    pub key: FranchiseKey,
    /// Display title: the component's earliest-year title (falling
    /// back to the lowest-id member's, then the parsed name).
    pub title: String,
    /// Member series, sorted by (year, id) — the season order proxy
    /// until something better exists.
    pub series: Vec<AniDbSeriesId>,
    /// First air year across members, for sorting.
    pub year: Option<u16>,
    /// Files known to belong to this franchise (via metadata).
    pub files: Vec<Ed2kHash>,
}

/// Group everything the state knows about into franchises.
pub fn franchises(view: &StateView) -> Vec<Franchise> {
    // Union-find over series ids: every relation edge connects.
    let mut parent: BTreeMap<AniDbSeriesId, AniDbSeriesId> = BTreeMap::new();
    fn find(
        parent: &mut BTreeMap<AniDbSeriesId, AniDbSeriesId>,
        id: AniDbSeriesId,
    ) -> AniDbSeriesId {
        let mut root = id;
        while let Some(&next) = parent.get(&root) {
            if next == root {
                break;
            }
            root = next;
        }
        parent.insert(id, root);
        root
    }
    fn union(
        parent: &mut BTreeMap<AniDbSeriesId, AniDbSeriesId>,
        a: AniDbSeriesId,
        b: AniDbSeriesId,
    ) {
        let (ra, rb) = (find(parent, a), find(parent, b));
        let (lo, hi) = if ra <= rb { (ra, rb) } else { (rb, ra) };
        parent.insert(hi, lo);
    }

    for (series, relations) in &view.series_relations {
        find(&mut parent, *series);
        for relation in &relations.relations {
            // Only structural edges (sequel chains, remakes, spin-offs)
            // merge franchises; crossover/shared-universe edges link
            // separate works and would otherwise collapse them all into
            // one component (e.g. Isekai Quartet -> every show it crosses
            // over). See `RelationKind::groups_franchise`.
            if relation.kind.groups_franchise() {
                union(&mut parent, *series, relation.target);
            }
        }
    }
    // Series known only through file metadata still form (singleton)
    // components.
    for metadata in view.anidb_metadata.values().flatten() {
        if let Some(series) = metadata.series_id {
            find(&mut parent, series);
        }
    }

    // Collect components.
    let ids: Vec<AniDbSeriesId> = parent.keys().copied().collect();
    let mut components: BTreeMap<AniDbSeriesId, BTreeSet<AniDbSeriesId>> = BTreeMap::new();
    for id in ids {
        let root = find(&mut parent, id);
        components.entry(root).or_default().insert(id);
    }

    // Series that actually have a known file (via metadata). The
    // relations walk pulls in the whole graph -- sequels you don't have,
    // standalone shows reached through crossovers -- so a series with no
    // file exists only as a relation target. Such series are dropped
    // from the members list, and a franchise with no files at all does
    // not appear. (Title/year are still computed from the full
    // component, so "Overlord" stays the name even when only a later
    // season is held.)
    let series_with_files: BTreeSet<AniDbSeriesId> = view
        .anidb_metadata
        .values()
        .flatten()
        .filter_map(|metadata| metadata.series_id)
        .collect();

    let mut result: Vec<Franchise> = components
        .into_iter()
        .filter_map(|(root, members)| {
            // Title/year from the earliest-year member with relations
            // data; fall back to the lowest id's title, then a stub.
            let mut best: Option<(&str, Option<u16>)> = None;
            let mut year_min: Option<u16> = None;
            for member in &members {
                if let Some(relations) = view.series_relations.get(member) {
                    let year = relations.year;
                    year_min = match (year_min, year) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                    let better = match (&best, year) {
                        (None, _) => true,
                        (Some((_, Some(existing))), Some(candidate)) => candidate < *existing,
                        (Some((_, None)), Some(_)) => true,
                        _ => false,
                    };
                    if better {
                        best = Some((&relations.title, year));
                    }
                }
            }
            let title = best
                .map(|(title, _)| title.to_string())
                .or_else(|| {
                    // No relations data at all: borrow a metadata name.
                    view.anidb_metadata.values().flatten().find_map(|metadata| {
                        (metadata.series_id.is_some_and(|id| members.contains(&id)))
                            .then(|| metadata.series_name.clone())
                    })
                })
                .unwrap_or_else(|| format!("anidb:{}", root.0));
            let files: Vec<Ed2kHash> = view
                .anidb_metadata
                .iter()
                .filter_map(|(hash, metadata)| {
                    let metadata = metadata.as_ref()?;
                    metadata
                        .series_id
                        .is_some_and(|id| members.contains(&id))
                        .then_some(*hash)
                })
                .collect();
            // A franchise reached only through relations holds no files.
            if files.is_empty() {
                return None;
            }
            let mut series: Vec<AniDbSeriesId> = members
                .iter()
                .copied()
                .filter(|id| series_with_files.contains(id))
                .collect();
            series.sort_by_key(|id| {
                (
                    view.series_relations
                        .get(id)
                        .and_then(|r| r.year)
                        .unwrap_or(u16::MAX),
                    id.0,
                )
            });
            Some(Franchise {
                key: FranchiseKey::Series(root),
                title,
                series,
                year: year_min,
                files,
            })
        })
        .collect();

    // Name-fallback groups for files with metadata but no series id.
    let mut by_name: BTreeMap<String, Vec<Ed2kHash>> = BTreeMap::new();
    for (hash, metadata) in &view.anidb_metadata {
        if let Some(metadata) = metadata
            && metadata.series_id.is_none()
        {
            by_name
                .entry(metadata.series_name.clone())
                .or_default()
                .push(*hash);
        }
    }
    for (name, files) in by_name {
        result.push(Franchise {
            key: FranchiseKey::Name(name.clone()),
            title: name,
            series: Vec::new(),
            year: None,
            files,
        });
    }

    result.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.key.cmp(&b.key)));
    result
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::state::CrdtState;
    use crate::types::{
        ActorId, AniDbMetadata, MetadataSource, RelationKind, SeriesRelation, SeriesRelations,
        SharedTimestamp,
    };

    fn ts(t: u64) -> SharedTimestamp {
        SharedTimestamp(t)
    }

    fn relations(
        title: &str,
        year: Option<u16>,
        targets: &[(RelationKind, u32)],
    ) -> SeriesRelations {
        SeriesRelations {
            title: title.into(),
            year,
            episode_count: Some(12),
            relations: targets
                .iter()
                .map(|(kind, id)| SeriesRelation {
                    kind: *kind,
                    target: AniDbSeriesId(*id),
                })
                .collect(),
        }
    }

    fn metadata(name: &str, series: Option<u32>) -> AniDbMetadata {
        AniDbMetadata {
            source: if series.is_some() {
                MetadataSource::AniDb
            } else {
                MetadataSource::FilenameDerived
            },
            series_name: name.into(),
            series_id: series.map(AniDbSeriesId),
            episode_number: Some("1".into()),
        }
    }

    #[test]
    fn sequels_group_with_earliest_title_and_files_attach() {
        let mut state = CrdtState::new();
        let a = ActorId::SERVER;
        // Season 1 (2020) <-> Season 2 (2022); unrelated other show.
        state.set_series_relations(
            a,
            ts(1),
            AniDbSeriesId(1),
            relations("Frieren", Some(2020), &[(RelationKind::Sequel, 2)]),
        );
        state.set_series_relations(
            a,
            ts(2),
            AniDbSeriesId(2),
            relations("Frieren S2", Some(2022), &[(RelationKind::Prequel, 1)]),
        );
        state.set_series_relations(
            a,
            ts(3),
            AniDbSeriesId(9),
            relations("Lain", Some(1998), &[]),
        );
        state.set_anidb_metadata(
            a,
            ts(4),
            Ed2kHash([1; 16]),
            Some(metadata("Frieren S2", Some(2))),
        );
        state.set_anidb_metadata(
            a,
            ts(5),
            Ed2kHash([2; 16]),
            Some(metadata("Parsed Show", None)),
        );

        let groups = franchises(&state.view());
        // Lain has no file -> dropped (relation-only). Two remain.
        assert_eq!(groups.len(), 2, "{groups:#?}");

        let frieren = groups.iter().find(|f| f.title == "Frieren").unwrap();
        // Only S2 is held; S1 is filtered out as a file-less member, but
        // the franchise title/year still come from the full component.
        assert_eq!(frieren.series, vec![AniDbSeriesId(2)]);
        assert_eq!(frieren.year, Some(2020));
        assert_eq!(frieren.files, vec![Ed2kHash([1; 16])]);

        let parsed = groups.iter().find(|f| f.title == "Parsed Show").unwrap();
        assert_eq!(parsed.key, FranchiseKey::Name("Parsed Show".into()));
        assert_eq!(parsed.files, vec![Ed2kHash([2; 16])]);
    }

    #[test]
    fn metadata_only_series_is_a_singleton_franchise() {
        let mut state = CrdtState::new();
        state.set_anidb_metadata(
            ActorId::SERVER,
            ts(1),
            Ed2kHash([3; 16]),
            Some(metadata("Lonely", Some(7))),
        );
        let groups = franchises(&state.view());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].title, "Lonely");
        assert_eq!(groups[0].key, FranchiseKey::Series(AniDbSeriesId(7)));
    }

    /// Regression (real AniDB data): a crossover anime like Isekai
    /// Quartet relates to Overlord, KonoSuba, Re:Zero and Youjo Senki via
    /// the "Other"/crossover relation code. Those are four independent
    /// franchises; the crossover must not collapse them into one giant
    /// component. Before the structural-only filter this merged all four.
    #[test]
    fn crossover_does_not_merge_franchises() {
        let mut state = CrdtState::new();
        let a = ActorId::SERVER;
        // Four standalone shows, each its own franchise.
        for (i, (id, name, year)) in [
            (10816, "Overlord", 2015),
            (11261, "KonoSuba", 2016),
            (11370, "Re:Zero", 2016),
            (11905, "Youjo Senki", 2017),
        ]
        .into_iter()
        .enumerate()
        {
            state.set_series_relations(
                a,
                ts(i as u64 + 1),
                AniDbSeriesId(id),
                relations(name, Some(year), &[]),
            );
            // A held file per show, so each survives the file-less filter.
            state.set_anidb_metadata(
                a,
                ts(i as u64 + 20),
                Ed2kHash([i as u8 + 1; 16]),
                Some(metadata(name, Some(id))),
            );
        }
        // The crossover, linked to all four via the crossover code (100).
        state.set_series_relations(
            a,
            ts(10),
            AniDbSeriesId(14435),
            relations(
                "Isekai Quartet",
                Some(2019),
                &[
                    (RelationKind::Other(100), 10816),
                    (RelationKind::Other(100), 11261),
                    (RelationKind::Other(100), 11370),
                    (RelationKind::Other(100), 11905),
                ],
            ),
        );
        state.set_anidb_metadata(
            a,
            ts(30),
            Ed2kHash([42; 16]),
            Some(metadata("Isekai Quartet", Some(14435))),
        );

        let groups = franchises(&state.view());
        // Four standalone shows + the crossover, all separate.
        assert_eq!(groups.len(), 5, "{groups:#?}");
        for f in &groups {
            assert_eq!(f.series.len(), 1, "no franchise should absorb others: {f:#?}");
        }
    }

    /// The relations walk pulls in the whole graph -- sequels you don't
    /// have, standalone shows reached via crossovers -- so series that
    /// exist *only* as relation targets carry no files. The browser must
    /// show only series the group has actually touched: a file-bearing
    /// franchise keeps only its file-bearing members, and a franchise
    /// with no files at all does not appear.
    #[test]
    fn relation_only_series_are_filtered_out() {
        let mut state = CrdtState::new();
        let a = ActorId::SERVER;
        // Overlord I (no file) -> ... -> Overlord IV (the only file).
        state.set_series_relations(
            a,
            ts(1),
            AniDbSeriesId(10816),
            relations("Overlord", Some(2015), &[(RelationKind::Sequel, 16296)]),
        );
        state.set_series_relations(
            a,
            ts(2),
            AniDbSeriesId(16296),
            relations("Overlord IV", Some(2022), &[(RelationKind::Prequel, 10816)]),
        );
        // KonoSuba: known only through the relations walk, no files.
        state.set_series_relations(
            a,
            ts(3),
            AniDbSeriesId(11261),
            relations("KonoSuba", Some(2016), &[]),
        );
        state.set_anidb_metadata(
            a,
            ts(4),
            Ed2kHash([1; 16]),
            Some(metadata("Overlord IV", Some(16296))),
        );

        let groups = franchises(&state.view());
        // KonoSuba (no files) is gone; only the Overlord franchise remains.
        assert_eq!(groups.len(), 1, "{groups:#?}");
        let overlord = &groups[0];
        // Title still reflects the franchise root, not just the season held.
        assert_eq!(overlord.title, "Overlord");
        // The file-less Overlord I season is filtered from the members.
        assert_eq!(overlord.series, vec![AniDbSeriesId(16296)]);
        assert_eq!(overlord.files, vec![Ed2kHash([1; 16])]);
    }

    /// Spec: only *structural* relation kinds (sequel/prequel chains,
    /// alternative versions, side/parent/summary/full stories) place two
    /// series in the same franchise. Setting/character/music-video and
    /// the catch-all crossover code link related-but-separate works and
    /// must not group. One edge of each kind, between two otherwise
    /// disconnected series, is the cleanest probe.
    #[test]
    fn only_structural_relations_group() {
        let cases = [
            (RelationKind::Sequel, true),
            (RelationKind::Prequel, true),
            (RelationKind::AlternativeVersion, true),
            (RelationKind::SideStory, true),
            (RelationKind::ParentStory, true),
            (RelationKind::Summary, true),
            (RelationKind::FullStory, true),
            (RelationKind::SameSetting, false),
            (RelationKind::AlternativeSetting, false),
            (RelationKind::MusicVideo, false),
            (RelationKind::Character, false),
            (RelationKind::Other(100), false),
            (RelationKind::Other(0), false),
        ];
        for (kind, grouped) in cases {
            let mut state = CrdtState::new();
            let a = ActorId::SERVER;
            state.set_series_relations(
                a,
                ts(1),
                AniDbSeriesId(1),
                relations("A", Some(2000), &[(kind, 2)]),
            );
            state.set_series_relations(a, ts(2), AniDbSeriesId(2), relations("B", Some(2001), &[]));
            // A held file for each, so neither is filtered as file-less.
            state.set_anidb_metadata(a, ts(3), Ed2kHash([1; 16]), Some(metadata("A", Some(1))));
            state.set_anidb_metadata(a, ts(4), Ed2kHash([2; 16]), Some(metadata("B", Some(2))));
            let groups = franchises(&state.view());
            let expected = if grouped { 1 } else { 2 };
            assert_eq!(
                groups.len(),
                expected,
                "kind {kind:?}: expected {expected} franchise(s), got {groups:#?}"
            );
        }
    }
}
