//! State compaction: rebuilding a [`CrdtState`] from its resolved view
//! under a single actor.
//!
//! This is the server's daily maintenance pass (docs/sync-state.md,
//! Compaction), kept here as a pure function so it can be property
//! tested: it must preserve the resolved view *exactly*, except for the
//! documented reductions (chat trimmed, lookup set cleared, playlist
//! positions rebalanced, tombstones purged).
//!
//! Rebuilding under one actor is also what keeps per-session actor
//! clocks from accumulating forever (see `ActorId::session`): the
//! fresh state's maps know only the rebuilding actor.

use crate::playlist::NewPlaylistEntry;
use crate::state::{CrdtState, StateView};
use crate::types::{ActorId, SharedTimestamp};

/// Rebuild `view` as a fresh state authored entirely by `actor`.
///
/// `stamp` supplies write timestamps; it must be monotonic and its
/// first value must exceed every timestamp in the source state (the
/// Lamport floor), or later writes from live clients could lose LWW
/// races against resurrected old values.
///
/// Reductions, by design:
/// - playlist tombstones are gone (the view never contained them) and
///   positions are rebalanced to small flat identifiers;
/// - watched flags for files no longer on the playlist are dropped;
/// - chat keeps only the trailing `chat_keep` messages (archive the
///   full log first);
/// - the lookup-request set empties (clients re-request what still
///   matters).
pub fn rebuild(
    view: &StateView,
    actor: ActorId,
    chat_keep: usize,
    mut stamp: impl FnMut() -> SharedTimestamp,
) -> CrdtState {
    let mut fresh = CrdtState::new();

    for entry in &view.playlist {
        fresh.push_playlist_entry(
            actor,
            stamp(),
            NewPlaylistEntry {
                hash: entry.hash,
                added_by: entry.state.added_by.clone(),
                filename: entry.state.filename.clone(),
                size_bytes: entry.state.size_bytes,
                duration_millis: entry.state.duration_millis,
            },
        );
    }
    for (hash, watched) in &view.watched {
        if view.playlist.iter().any(|entry| entry.hash == *hash) {
            fresh.set_watched(actor, stamp(), *hash, *watched);
        }
    }
    fresh.set_now_playing(actor, stamp(), view.now_playing);
    if let Some(authority) = &view.seek_authority {
        fresh.set_seek_authority(actor, stamp(), authority.clone());
    }
    fresh.set_playback_intent(actor, stamp(), view.playback_intent);
    for ((user, series), pref) in &view.series_preference {
        fresh.set_series_preference(actor, stamp(), user.clone(), *series, *pref);
    }
    for (user, value) in &view.manual_override {
        fresh.set_manual_override(actor, stamp(), user.clone(), value.clone());
    }
    for ((user, file), availability) in &view.file_availability {
        fresh.set_file_availability(actor, stamp(), user.clone(), *file, *availability);
    }
    for (hash, metadata) in &view.anidb_metadata {
        fresh.set_anidb_metadata(actor, stamp(), *hash, metadata.clone());
    }
    for (series, relations) in &view.series_relations {
        fresh.set_series_relations(actor, stamp(), *series, relations.clone());
    }
    for (hash, entry) in &view.file_catalog {
        fresh.set_file_catalog(actor, stamp(), *hash, entry.clone());
    }
    for (id, entry) in &view.list_entries {
        fresh.put_list_entry(actor, stamp(), *id, entry.clone());
    }
    for (id, next_ep) in &view.list_next_ep {
        fresh.set_next_ep(actor, stamp(), *id, next_ep.clone());
    }
    for message in view.chat.iter().rev().take(chat_keep).rev() {
        fresh.append_chat(message.clone());
    }
    for (user, position) in &view.playback_position {
        fresh.set_playback_position(actor, stamp(), user.clone(), *position);
    }

    fresh
}
