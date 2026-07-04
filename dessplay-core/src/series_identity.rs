//! Resolving a file to its Series Identity List entry (design.md, Series
//! Identity): AniDB linking is enrichment only, never a prerequisite for
//! commitment or gating -- every committable series routes through its
//! [`SeriesListEntry`](crate::types::SeriesListEntry) instead.

use std::collections::BTreeSet;

use crate::state::StateView;
use crate::types::{Ed2kHash, ListEntryId, ListStatus, SeriesListEntry};

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
///
/// This crate does no randomness in non-test code (see `lib.rs`'s pure,
/// I/O-free scope), so it hands back the entry *without* an id -- the
/// caller (which has `rand`) mints a `ListEntryId` and issues the
/// `PutListEntry` mutation alongside whatever action triggered creation.
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
