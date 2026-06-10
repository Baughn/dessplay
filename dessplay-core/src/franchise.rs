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
            union(&mut parent, *series, relation.target);
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

    let mut result: Vec<Franchise> = components
        .into_iter()
        .map(|(root, members)| {
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
            let mut series: Vec<AniDbSeriesId> = members.iter().copied().collect();
            series.sort_by_key(|id| {
                (
                    view.series_relations
                        .get(id)
                        .and_then(|r| r.year)
                        .unwrap_or(u16::MAX),
                    id.0,
                )
            });
            let files = view
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
            Franchise {
                key: FranchiseKey::Series(root),
                title,
                series,
                year: year_min,
                files,
            }
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
        assert_eq!(groups.len(), 3, "{groups:#?}");

        let frieren = groups.iter().find(|f| f.title == "Frieren").unwrap();
        assert_eq!(frieren.series, vec![AniDbSeriesId(1), AniDbSeriesId(2)]);
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
}
