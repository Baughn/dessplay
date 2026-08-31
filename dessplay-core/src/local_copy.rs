//! Candidate selection for the local-copy offer (proposal
//! 2026-08-31-local-copy-offer): when now-playing resolves locally
//! Missing for a client with auto-download disabled, rank the library
//! files that plausibly *are* this episode in another encode, so the
//! user can map one instead of hand-browsing.
//!
//! Two evidence classes, strong first:
//!
//! 1. **Same episode**: the candidate's synced metadata carries the same
//!    `(series_id, parsed episode number)` as the target entry's — the
//!    episode browser's copy-grouping equivalence.
//! 2. **Name match**: the candidate has *no* episode identity (no
//!    metadata, or metadata without a series id / parseable epno) and its
//!    filename is within [`MAX_NAME_DISTANCE`] of the target's after
//!    normalization — guarded by the filename episode parse, because pure
//!    Levenshtein rates `… - 01` vs `… - 02` at distance 1 and would rank
//!    the wrong episode above a `v2` rename of the right one.
//!
//! A file whose metadata names a *different* episode is never offered,
//! however close its name: it is positively known to be the wrong one.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::episode_parse;
use crate::state::StateView;
use crate::types::{AniDbSeriesId, Ed2kHash};

/// Maximum normalized Levenshtein distance for a name-match candidate.
/// Generous enough to admit a `v2` marker or a changed 8-char CRC tag;
/// the episode-number guard screens the adjacent-episode failure mode
/// that would otherwise make this threshold dangerous.
pub const MAX_NAME_DISTANCE: usize = 8;

/// Why a candidate qualifies. Ordering ranks strong evidence first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum CopyEvidence {
    /// Same `(series_id, parsed episode number)` as the target.
    SameEpisode,
    /// No episode identity, filename within [`MAX_NAME_DISTANCE`].
    NameMatch,
}

/// One offerable local file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CopyCandidate {
    /// The local file (a library-index path).
    pub path: PathBuf,
    /// The path's final component, for display.
    pub filename: String,
    /// The candidate's ed2k root (its library-index identity).
    pub hash: Ed2kHash,
    /// Why it qualifies.
    pub evidence: CopyEvidence,
}

/// A file's episode identity: `(series_id, (category, number))` when the
/// synced metadata carries both, else `None`. Filename-derived fallback
/// metadata has no series id, so it yields `None` — such files are
/// name-match material, never same-episode evidence.
fn episode_identity(view: &StateView, hash: Ed2kHash) -> Option<(AniDbSeriesId, (u8, u64))> {
    let meta = view.anidb_metadata.get(&hash)?.as_ref()?;
    let series = meta.series_id?;
    let epno = episode_parse::parse_anidb_epno(meta.episode_number.as_deref())?;
    Some((series, epno))
}

/// Normalize a filename for the edit-distance comparison: lowercase,
/// spaces coerced to underscores (release names flip freely between the
/// two).
fn normalize_name(name: &str) -> String {
    name.to_lowercase().replace(' ', "_")
}

/// Rank the library files that plausibly hold the target entry's episode.
///
/// `library` is the live library index (`(path, ed2k root, mtime)` rows —
/// vanished-root rows are already excluded at the source). Returns
/// candidates strong-evidence-first, then by name distance, deduplicated
/// by hash (several paths to identical content offer once). Empty when
/// the target is not a playlist entry.
pub fn local_copy_candidates(
    view: &StateView,
    target: Ed2kHash,
    library: &[(PathBuf, Ed2kHash, i64)],
) -> Vec<CopyCandidate> {
    let Some(entry) = view.playlist.iter().find(|e| e.hash == target) else {
        return Vec::new();
    };
    let target_name = entry.state.filename.as_str();
    let target_normalized = normalize_name(target_name);
    let target_identity = episode_identity(view, target);
    let target_episode = episode_parse::parse_episode_number(target_name);

    // Dedupe by hash: identical content under several paths offers once
    // (keep the lexicographically-first path, deterministically).
    let mut by_hash: BTreeMap<Ed2kHash, (CopyCandidate, usize)> = BTreeMap::new();
    for (path, hash, _mtime) in library {
        let hash = *hash;
        if hash == target {
            // A copy of the target itself would have resolved the entry.
            continue;
        }
        let Some(filename) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let distance = strsim::levenshtein(&normalize_name(&filename), &target_normalized);
        let evidence = match episode_identity(view, hash) {
            // Strong evidence: the same episode, whatever the name.
            Some(identity) if Some(identity) == target_identity => CopyEvidence::SameEpisode,
            // A *different* known episode is positively wrong — never
            // offered, however close the name (this also covers a target
            // with no identity: an identified candidate can't be tied to
            // it, so it only ever qualifies through branch 1).
            Some(_) => continue,
            None => {
                // Name-match branch: episode-number guard first. When
                // both filenames parse an episode number they must
                // agree — distance alone rates `- 01` vs `- 02` at 1.
                let candidate_episode = episode_parse::parse_episode_number(&filename);
                if let (Some(a), Some(b)) = (&candidate_episode, &target_episode)
                    && a != b
                {
                    continue;
                }
                if distance > MAX_NAME_DISTANCE {
                    continue;
                }
                CopyEvidence::NameMatch
            }
        };
        let candidate = CopyCandidate {
            path: path.clone(),
            filename,
            hash,
            evidence,
        };
        match by_hash.entry(hash) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert((candidate, distance));
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                if candidate.path < slot.get().0.path {
                    slot.insert((candidate, distance));
                }
            }
        }
    }

    let mut ranked: Vec<(CopyCandidate, usize)> = by_hash.into_values().collect();
    ranked.sort_by(|(a, da), (b, db)| {
        (a.evidence, da, &a.filename, &a.path).cmp(&(b.evidence, db, &b.filename, &b.path))
    });
    ranked.into_iter().map(|(candidate, _)| candidate).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::playlist::NewPlaylistEntry;
    use crate::state::CrdtState;
    use crate::types::{ActorId, AniDbMetadata, MetadataSource, SharedTimestamp, UserId};
    use proptest::prelude::*;

    const A: ActorId = ActorId(1);

    fn hash(i: u8) -> Ed2kHash {
        Ed2kHash([i; 16])
    }

    fn ts(t: u64) -> SharedTimestamp {
        SharedTimestamp(t)
    }

    fn metadata(series: Option<u32>, epno: Option<&str>) -> AniDbMetadata {
        AniDbMetadata {
            source: if series.is_some() {
                MetadataSource::AniDb
            } else {
                MetadataSource::FilenameDerived
            },
            series_name: "Higurashi".into(),
            series_id: series.map(AniDbSeriesId),
            episode_number: epno.map(String::from),
        }
    }

    /// A view with one playlist entry (`hash(1)`, `target_name`), plus
    /// per-hash metadata.
    fn view_with(
        target_name: &str,
        metadata_rows: &[(u8, Option<u32>, Option<&str>)],
    ) -> StateView {
        let mut state = CrdtState::new();
        state.push_playlist_entry(
            A,
            ts(1),
            NewPlaylistEntry {
                hash: hash(1),
                added_by: UserId::new("baughn"),
                filename: target_name.into(),
                size_bytes: 1000,
                duration_millis: None,
            },
        );
        for (t, (h, series, epno)) in (10..).zip(metadata_rows.iter()) {
            state.set_anidb_metadata(A, ts(t), hash(*h), Some(metadata(*series, *epno)));
        }
        state.view()
    }

    fn lib(rows: &[(u8, &str)]) -> Vec<(PathBuf, Ed2kHash, i64)> {
        rows.iter()
            .map(|(h, name)| (PathBuf::from(format!("/media/{name}")), hash(*h), 0))
            .collect()
    }

    const TARGET: &str =
        "[RESubs] Higurashi no Naku Koro Ni Kai - 01v2 (BD 1920x1080 AC3) [40B49C7B].mkv";

    #[test]
    fn same_episode_offered_regardless_of_name() {
        // Target and candidate share (series 500, ep 1) under unrelated names.
        let view = view_with(
            TARGET,
            &[(1, Some(500), Some("01")), (2, Some(500), Some("01"))],
        );
        let candidates =
            local_copy_candidates(&view, hash(1), &lib(&[(2, "totally-different-name.mkv")]));
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].evidence, CopyEvidence::SameEpisode);
        assert_eq!(candidates[0].hash, hash(2));
    }

    #[test]
    fn different_known_episode_never_offered() {
        // Same series, episode 2 — distance 1 from the target's name form,
        // but positively the wrong episode.
        let view = view_with(
            TARGET,
            &[(1, Some(500), Some("01")), (2, Some(500), Some("02"))],
        );
        let candidates = local_copy_candidates(&view, hash(1), &lib(&[(2, TARGET)]));
        assert!(candidates.is_empty());
    }

    #[test]
    fn identical_name_unknown_file_matches_at_distance_zero() {
        // The motivating Higurashi case: same filename, different ed2k,
        // AniDB knows only one of them.
        let view = view_with(TARGET, &[(1, Some(500), Some("01"))]);
        let candidates = local_copy_candidates(&view, hash(1), &lib(&[(2, TARGET)]));
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].evidence, CopyEvidence::NameMatch);
    }

    #[test]
    fn epno_guard_rejects_adjacent_episode() {
        // `- 01` vs `- 02` is Levenshtein distance 1; the filename parse
        // must veto it.
        let view = view_with("Show - 01.mkv", &[]);
        let candidates = local_copy_candidates(&view, hash(1), &lib(&[(2, "Show - 02.mkv")]));
        assert!(candidates.is_empty());
    }

    #[test]
    fn v2_rename_and_case_space_normalization_match() {
        let view = view_with("My Show - 01.mkv", &[]);
        let candidates = local_copy_candidates(&view, hash(1), &lib(&[(2, "my_show - 01v2.mkv")]));
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn distance_cap_excludes_unrelated_names() {
        let view = view_with("Show - 01.mkv", &[]);
        let candidates = local_copy_candidates(
            &view,
            hash(1),
            &lib(&[(2, "A Completely Unrelated Series - 01.mkv")]),
        );
        assert!(candidates.is_empty());
    }

    #[test]
    fn target_hash_and_duplicates_excluded() {
        let view = view_with(TARGET, &[]);
        // hash(1) is the target itself; hash(2) appears twice.
        let candidates = local_copy_candidates(
            &view,
            hash(1),
            &lib(&[(1, TARGET), (2, TARGET), (2, TARGET)]),
        );
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].hash, hash(2));
    }

    #[test]
    fn strong_evidence_ranks_first() {
        let view = view_with(
            TARGET,
            &[(1, Some(500), Some("01")), (3, Some(500), Some("01"))],
        );
        let candidates = local_copy_candidates(
            &view,
            hash(1),
            &lib(&[(2, TARGET), (3, "different-encode.mkv")]),
        );
        assert_eq!(
            candidates
                .iter()
                .map(|c| (c.hash, c.evidence))
                .collect::<Vec<_>>(),
            vec![
                (hash(3), CopyEvidence::SameEpisode),
                (hash(2), CopyEvidence::NameMatch),
            ]
        );
    }

    /// Filename fragments that stay parse-compatible: episode markers only
    /// via the generated suffix below.
    fn name_stem() -> impl Strategy<Value = String> {
        "[A-Za-z ]{1,20}".prop_map(|s| s.trim().to_string() + " x")
    }

    proptest! {
        /// The guard invariant: a candidate whose filename parses to a
        /// different episode number than the target's is never offered,
        /// whatever the distance.
        #[test]
        fn never_offers_a_parsed_different_episode(
            stem in name_stem(),
            target_ep in 1u8..=99,
            candidate_ep in 1u8..=99,
        ) {
            prop_assume!(target_ep != candidate_ep);
            let target = format!("{stem} - {target_ep:02}.mkv");
            let candidate = format!("{stem} - {candidate_ep:02}.mkv");
            let view = view_with(&target, &[]);
            let candidates =
                local_copy_candidates(&view, hash(1), &lib(&[(2, &candidate)]));
            prop_assert!(candidates.is_empty(), "offered {candidate:?} for {target:?}");
        }

        /// A same-named unknown file (the Higurashi shape) is always
        /// offered, and a same-episode metadata match is always offered
        /// regardless of either filename.
        #[test]
        fn always_offers_the_motivating_shapes(
            stem in name_stem(),
            other in name_stem(),
            ep in 1u8..=99,
        ) {
            let target = format!("{stem} - {ep:02}.mkv");
            // Same name, no metadata: branch 2, distance 0.
            let view = view_with(&target, &[]);
            let candidates = local_copy_candidates(&view, hash(1), &lib(&[(2, &target)]));
            prop_assert_eq!(candidates.len(), 1);

            // Same (series, epno), arbitrary other name: branch 1.
            let view = view_with(
                &target,
                &[(1, Some(500), Some("7")), (2, Some(500), Some("7"))],
            );
            let other_name = format!("{other}.mkv");
            let candidates =
                local_copy_candidates(&view, hash(1), &lib(&[(2, &other_name)]));
            prop_assert_eq!(candidates.len(), 1);
            prop_assert_eq!(candidates[0].evidence, CopyEvidence::SameEpisode);
        }
    }
}
