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
    let metadata = view.anidb_metadata.get(&file)?.as_ref()?;

    if let Some(series_id) = metadata.series_id
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

    view.list_entries
        .iter()
        .find(|(_, entry)| {
            entry.name == metadata.series_name
                || entry.local_aliases.contains(&metadata.series_name)
        })
        .map(|(id, _)| *id)
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

    /// Regression: two independent auto-create resolutions for the *same*
    /// AniDB series must converge on the same entry id. A caught-in-the-
    /// wild bug (`missing_unknown_series_auto_not_watching_lets_the_group_play`
    /// flaking under `-D warnings`/CI load): with a random id, two racing
    /// peers each auto-creating "unknown series -> NotWatching" for a file
    /// neither has watch history for would fork onto two different
    /// entries that never merge back into one commitment.
    #[test]
    fn derive_entry_id_is_deterministic_for_the_same_series() {
        let series = AniDbSeriesId(4242);
        assert_eq!(
            derive_entry_id(Some(series), "irrelevant when linked"),
            derive_entry_id(Some(series), "a different name entirely"),
        );
    }

    #[test]
    fn derive_entry_id_is_deterministic_for_the_same_name() {
        assert_eq!(
            derive_entry_id(None, "Some Obscure Show"),
            derive_entry_id(None, "Some Obscure Show"),
        );
    }

    #[test]
    fn derive_entry_id_differs_across_series_and_across_names() {
        assert_ne!(
            derive_entry_id(Some(AniDbSeriesId(1)), ""),
            derive_entry_id(Some(AniDbSeriesId(2)), ""),
        );
        assert_ne!(
            derive_entry_id(None, "Show A"),
            derive_entry_id(None, "Show B"),
        );
        // A linked id must never coincide with an unlinked one derived from
        // an unrelated name -- they hash under different domains.
        assert_ne!(
            derive_entry_id(Some(AniDbSeriesId(1)), ""),
            derive_entry_id(None, ""),
        );
    }

    /// The actual race this exists to close: two peers independently call
    /// `resolve_or_build_entry` for the same file/series before either's
    /// `PutListEntry` has round-tripped -- both must compute the same
    /// (id, entry) pair, not two different ids.
    #[test]
    fn resolve_or_build_entry_converges_when_called_twice_with_no_entry_yet() {
        let mut state = CrdtState::new();
        state.set_anidb_metadata(
            A,
            ts(1),
            hash(1),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Some Obscure Show".into(),
                series_id: Some(AniDbSeriesId(4242)),
                episode_number: Some("1".into()),
            }),
        );
        let view = state.view();
        let first = resolve_or_build_entry(&view, hash(1));
        let second = resolve_or_build_entry(&view, hash(1));
        assert_eq!(first.map(|(id, _)| id), second.map(|(id, _)| id));
    }
}
