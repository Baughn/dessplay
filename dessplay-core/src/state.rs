//! The combined CRDT state container and its wire operation type.
//!
//! All shared state lives here as `crdts` types. Mutations follow one
//! pattern: a mutator method generates the native op with the current
//! causal context, applies it locally (immediate feedback), and returns it
//! wrapped in [`CrdtOp`] for broadcast through the server. Remote ops are
//! applied with [`CrdtState::apply`]; reconnection sync uses
//! [`CrdtState::merge`] (CvRDT), which is idempotent, commutative, and
//! associative.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

use crdts::{CmRDT, CvRDT, GList, GSet, Map, glist};
use serde::{Deserialize, Serialize};

use crate::lww::{Lww, LwwCell, resolve_value};
use crate::types::{
    ActorId, AniDbMetadata, AniDbSeriesId, ChatMessage, Ed2kHash, Epoch, FileAvailability,
    FileHashInfo, ListEntryId, ManualState, NextEpState, PlaybackIntent, PlaybackPosition,
    PlaylistFileState, SeekAuthority, SeriesListEntry, SeriesRelations, SeriesWatchState,
    SharedTimestamp, UserId,
};

/// A keyed collection of LWW registers — the standard map shape.
pub type LwwMap<K, V> = Map<K, LwwCell<V>, ActorId>;

/// The native op type for an [`LwwMap`].
pub type LwwMapOp<K, V> = crdts::map::Op<K, LwwCell<V>, ActorId>;

/// The op type for a standalone [`LwwCell`]: the timestamped value
/// itself.
pub type LwwRegOp<V> = Lww<V>;

/// All replicated DessPlay state. See docs/sync-state.md for the full
/// table of types, owners, and conflict-resolution rationale.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CrdtState {
    /// The shared playlist, keyed by file hash. `None` is a removal
    /// tombstone: we never use `Map::rm` (see the `CrdtOp` docs), and the
    /// server purges tombstones at compaction.
    pub playlist: LwwMap<Ed2kHash, Option<PlaylistFileState>>,
    /// Group watched flags. Server-only writes, at EOF.
    pub watched: LwwMap<Ed2kHash, bool>,
    /// The currently playing file, if any.
    pub now_playing: LwwCell<Option<Ed2kHash>>,
    /// Whoever last seeked; everyone syncs to their position.
    pub seek_authority: LwwCell<SeekAuthority>,
    /// The group's play/pause latch. Unwritten resolves as `Paused`.
    pub playback_intent: LwwCell<PlaybackIntent>,
    /// Per-user, per-series watch preference.
    pub series_preference: LwwMap<(UserId, AniDbSeriesId), SeriesWatchState>,
    /// Per-user manual state override. `Away` is writable by anyone.
    pub manual_override: LwwMap<UserId, Option<ManualState>>,
    /// Per-user, per-file availability.
    pub file_availability: LwwMap<(UserId, Ed2kHash), FileAvailability>,
    /// Server-authoritative file metadata. `None` = not yet looked up.
    pub anidb_metadata: LwwMap<Ed2kHash, Option<AniDbMetadata>>,
    /// Server-authoritative franchise relations graph.
    pub series_relations: LwwMap<AniDbSeriesId, SeriesRelations>,
    /// The List: the group's permanent series tracker.
    pub list_entries: LwwMap<ListEntryId, SeriesListEntry>,
    /// Fast-changing progress fields for List entries.
    pub list_next_ep: LwwMap<ListEntryId, NextEpState>,
    /// Files clients want looked up on AniDB. Cleared at compaction.
    pub lookup_requests: GSet<FileHashInfo>,
    /// Chat log. Trimmed at compaction (server archives first).
    pub chat: GList<ChatMessage>,
    /// Per-user playback positions. High-frequency, datagram transport.
    pub playback_position: LwwMap<UserId, PlaybackPosition>,
}

/// One replicated operation, as sent over the wire (postcard-serialized).
/// Each variant wraps the native `crdts` op for the corresponding
/// [`CrdtState`] field, carrying its original causal context.
///
/// Every op is a put-style write — DessPlay never emits `Map::Rm`.
/// Removal via `Map::rm` is not view-convergent in `crdts` when a remove
/// races a concurrent re-add (the entry-scoped remove clock interacts
/// badly with map-global put clocks, leaving ghost values on some
/// replicas but not others — found by property testing). Removals are
/// LWW tombstones instead; with puts only, convergence needs nothing
/// stronger than per-origin FIFO delivery, which the server hub
/// provides.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CrdtOp {
    /// Playlist entry put (add, move, or tombstone).
    Playlist(LwwMapOp<Ed2kHash, Option<PlaylistFileState>>),
    /// Watched flag write.
    Watched(LwwMapOp<Ed2kHash, bool>),
    /// Now-playing register write.
    NowPlaying(LwwRegOp<Option<Ed2kHash>>),
    /// Seek-authority register write.
    SeekAuthority(LwwRegOp<SeekAuthority>),
    /// Playback-intent register write.
    PlaybackIntent(LwwRegOp<PlaybackIntent>),
    /// Series preference write.
    SeriesPreference(LwwMapOp<(UserId, AniDbSeriesId), SeriesWatchState>),
    /// Manual override write.
    ManualOverride(LwwMapOp<UserId, Option<ManualState>>),
    /// File availability write.
    FileAvailability(LwwMapOp<(UserId, Ed2kHash), FileAvailability>),
    /// AniDB metadata write (server).
    AniDbMetadata(LwwMapOp<Ed2kHash, Option<AniDbMetadata>>),
    /// Series relations write (server).
    SeriesRelations(LwwMapOp<AniDbSeriesId, SeriesRelations>),
    /// List entry put/remove.
    ListEntry(LwwMapOp<ListEntryId, SeriesListEntry>),
    /// List next-episode write.
    ListNextEp(LwwMapOp<ListEntryId, NextEpState>),
    /// Lookup request insert (GSet ops are the element itself).
    LookupRequest(FileHashInfo),
    /// Chat insert.
    Chat(glist::Op<ChatMessage>),
    /// Playback position write.
    PlaybackPosition(LwwMapOp<UserId, PlaybackPosition>),
}

impl CrdtOp {
    /// The LWW write timestamp embedded in this op, if it has one
    /// (chat and lookup-set inserts don't compete in any register).
    /// Receivers feed this into their Lamport floor so their next
    /// stamp dominates everything they have seen — see
    /// [`crate::lww`]'s module docs on timestamp discipline.
    pub fn lww_timestamp(&self) -> Option<SharedTimestamp> {
        fn map_ts<K, V>(op: &LwwMapOp<K, V>) -> Option<SharedTimestamp>
        where
            K: Ord + Clone + Debug,
            V: Ord + Clone + Debug,
        {
            match op {
                crdts::map::Op::Up { op, .. } => Some(op.timestamp),
                crdts::map::Op::Rm { .. } => None,
            }
        }
        match self {
            CrdtOp::Playlist(op) => map_ts(op),
            CrdtOp::Watched(op) => map_ts(op),
            CrdtOp::SeriesPreference(op) => map_ts(op),
            CrdtOp::ManualOverride(op) => map_ts(op),
            CrdtOp::FileAvailability(op) => map_ts(op),
            CrdtOp::AniDbMetadata(op) => map_ts(op),
            CrdtOp::SeriesRelations(op) => map_ts(op),
            CrdtOp::ListEntry(op) => map_ts(op),
            CrdtOp::ListNextEp(op) => map_ts(op),
            CrdtOp::PlaybackPosition(op) => map_ts(op),
            CrdtOp::NowPlaying(op) => Some(op.timestamp),
            CrdtOp::SeekAuthority(op) => Some(op.timestamp),
            CrdtOp::PlaybackIntent(op) => Some(op.timestamp),
            CrdtOp::LookupRequest(_) | CrdtOp::Chat(_) => None,
        }
    }
}

/// A full-state snapshot, as sent on reconnection or after compaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// The server's compaction generation.
    pub epoch: Epoch,
    /// The complete state.
    pub state: CrdtState,
}

/// Write the LWW winner for `key`. The map-level dot (from `actor`)
/// exists for `Map`'s per-origin dedup; the value op carries only the
/// timestamped value.
fn map_put<K, V>(
    map: &mut LwwMap<K, V>,
    actor: ActorId,
    ts: SharedTimestamp,
    key: K,
    value: V,
) -> LwwMapOp<K, V>
where
    K: Ord + Clone + Debug,
    V: Ord + Clone + Debug,
{
    let add_ctx = map.read_ctx().derive_add_ctx(actor);
    let op = map.update(key, add_ctx, |cell, _ctx| cell.write(ts, value));
    map.apply(op.clone());
    op
}

/// Write a standalone LWW register.
fn reg_put<V>(reg: &mut LwwCell<V>, ts: SharedTimestamp, value: V) -> LwwRegOp<V>
where
    V: Ord + Clone,
{
    let op = reg.write(ts, value);
    reg.apply(op.clone());
    op
}

/// Resolve every entry of an [`LwwMap`] to its LWW winner.
fn map_view<K, V>(map: &LwwMap<K, V>) -> BTreeMap<K, V>
where
    K: Ord + Clone,
    V: Ord + Clone,
{
    map.iter()
        .filter_map(|entry| {
            let (key, reg) = entry.val;
            resolve_value(reg).map(|value| (key.clone(), value))
        })
        .collect()
}

impl CrdtState {
    /// An empty state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one replicated operation (local echo or remote broadcast).
    /// Idempotent: re-applying a seen op is a no-op.
    pub fn apply(&mut self, op: CrdtOp) {
        match op {
            CrdtOp::Playlist(op) => self.playlist.apply(op),
            CrdtOp::Watched(op) => self.watched.apply(op),
            CrdtOp::NowPlaying(op) => self.now_playing.apply(op),
            CrdtOp::SeekAuthority(op) => self.seek_authority.apply(op),
            CrdtOp::PlaybackIntent(op) => self.playback_intent.apply(op),
            CrdtOp::SeriesPreference(op) => self.series_preference.apply(op),
            CrdtOp::ManualOverride(op) => self.manual_override.apply(op),
            CrdtOp::FileAvailability(op) => self.file_availability.apply(op),
            CrdtOp::AniDbMetadata(op) => self.anidb_metadata.apply(op),
            CrdtOp::SeriesRelations(op) => self.series_relations.apply(op),
            CrdtOp::ListEntry(op) => self.list_entries.apply(op),
            CrdtOp::ListNextEp(op) => self.list_next_ep.apply(op),
            CrdtOp::LookupRequest(info) => self.lookup_requests.apply(info),
            CrdtOp::Chat(op) => self.chat.apply(op),
            CrdtOp::PlaybackPosition(op) => self.playback_position.apply(op),
        }
    }

    /// CvRDT merge: fold another replica's full state into ours.
    /// Idempotent, commutative, associative — safe to apply at any time.
    pub fn merge(&mut self, other: CrdtState) {
        self.playlist.merge(other.playlist);
        self.watched.merge(other.watched);
        self.now_playing.merge(other.now_playing);
        self.seek_authority.merge(other.seek_authority);
        self.playback_intent.merge(other.playback_intent);
        self.series_preference.merge(other.series_preference);
        self.manual_override.merge(other.manual_override);
        self.file_availability.merge(other.file_availability);
        self.anidb_metadata.merge(other.anidb_metadata);
        self.series_relations.merge(other.series_relations);
        self.list_entries.merge(other.list_entries);
        self.list_next_ep.merge(other.list_next_ep);
        self.lookup_requests.merge(other.lookup_requests);
        self.chat.merge(other.chat);
        self.playback_position.merge(other.playback_position);
    }

    // ---- Mutators. Each applies locally and returns the op to broadcast.

    /// Put a playlist entry (add or rewrite, e.g. a move). Most callers
    /// want the position-aware helpers in the playlist module instead.
    pub fn set_playlist_entry(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        hash: Ed2kHash,
        entry: PlaylistFileState,
    ) -> CrdtOp {
        CrdtOp::Playlist(map_put(&mut self.playlist, actor, ts, hash, Some(entry)))
    }

    /// Remove a playlist entry by writing a tombstone. Pure LWW: a
    /// concurrent update with a later timestamp wins over the removal
    /// (and vice versa); either way every replica agrees. Tombstones are
    /// purged at compaction.
    pub fn remove_playlist_entry(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        hash: Ed2kHash,
    ) -> CrdtOp {
        CrdtOp::Playlist(map_put(&mut self.playlist, actor, ts, hash, None))
    }

    /// Set a file's group watched flag. Server-only by convention.
    pub fn set_watched(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        hash: Ed2kHash,
        watched: bool,
    ) -> CrdtOp {
        CrdtOp::Watched(map_put(&mut self.watched, actor, ts, hash, watched))
    }

    /// Set the now-playing file. (`actor` is unused — standalone LWW
    /// registers carry no causal metadata — but kept for mutator-API
    /// uniformity.)
    pub fn set_now_playing(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        file: Option<Ed2kHash>,
    ) -> CrdtOp {
        let _ = actor;
        CrdtOp::NowPlaying(reg_put(&mut self.now_playing, ts, file))
    }

    /// Take or hand over seek authority. (See [`Self::set_now_playing`]
    /// on the unused `actor`.)
    pub fn set_seek_authority(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        authority: SeekAuthority,
    ) -> CrdtOp {
        let _ = actor;
        CrdtOp::SeekAuthority(reg_put(&mut self.seek_authority, ts, authority))
    }

    /// Write the playback intent. Users on play/pause; the server on
    /// Lost, graceful quit, departure, and EOF-advance (always
    /// `Paused`). (See [`Self::set_now_playing`] on the unused `actor`.)
    pub fn set_playback_intent(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        intent: PlaybackIntent,
    ) -> CrdtOp {
        let _ = actor;
        CrdtOp::PlaybackIntent(reg_put(&mut self.playback_intent, ts, intent))
    }

    /// Set a user's watch preference for a series.
    pub fn set_series_preference(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        user: UserId,
        series: AniDbSeriesId,
        pref: SeriesWatchState,
    ) -> CrdtOp {
        CrdtOp::SeriesPreference(map_put(
            &mut self.series_preference,
            actor,
            ts,
            (user, series),
            pref,
        ))
    }

    /// Set a user's manual override. Owning user only — except `Away`,
    /// which anyone may write.
    pub fn set_manual_override(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        user: UserId,
        state: Option<ManualState>,
    ) -> CrdtOp {
        CrdtOp::ManualOverride(map_put(&mut self.manual_override, actor, ts, user, state))
    }

    /// Set a user's availability for a file.
    pub fn set_file_availability(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        user: UserId,
        file: Ed2kHash,
        availability: FileAvailability,
    ) -> CrdtOp {
        CrdtOp::FileAvailability(map_put(
            &mut self.file_availability,
            actor,
            ts,
            (user, file),
            availability,
        ))
    }

    /// Write file metadata. Server-only by convention.
    pub fn set_anidb_metadata(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        hash: Ed2kHash,
        metadata: Option<AniDbMetadata>,
    ) -> CrdtOp {
        CrdtOp::AniDbMetadata(map_put(&mut self.anidb_metadata, actor, ts, hash, metadata))
    }

    /// Write a series' relations. Server-only by convention.
    pub fn set_series_relations(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        series: AniDbSeriesId,
        relations: SeriesRelations,
    ) -> CrdtOp {
        CrdtOp::SeriesRelations(map_put(
            &mut self.series_relations,
            actor,
            ts,
            series,
            relations,
        ))
    }

    /// Create or edit a List entry (whole-struct LWW).
    pub fn put_list_entry(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        id: ListEntryId,
        entry: SeriesListEntry,
    ) -> CrdtOp {
        CrdtOp::ListEntry(map_put(&mut self.list_entries, actor, ts, id, entry))
    }

    /// Update a List entry's next-episode progress.
    pub fn set_next_ep(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        id: ListEntryId,
        next_ep: NextEpState,
    ) -> CrdtOp {
        CrdtOp::ListNextEp(map_put(&mut self.list_next_ep, actor, ts, id, next_ep))
    }

    /// Ask the server to look up a file on AniDB.
    pub fn request_lookup(&mut self, info: FileHashInfo) -> CrdtOp {
        self.lookup_requests.apply(info.clone());
        CrdtOp::LookupRequest(info)
    }

    /// Append a chat message.
    pub fn append_chat(&mut self, message: ChatMessage) -> CrdtOp {
        let op = self.chat.insert_after(self.chat.last(), message);
        self.chat.apply(op.clone());
        CrdtOp::Chat(op)
    }

    /// Report a user's playback position.
    pub fn set_playback_position(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        user: UserId,
        position: PlaybackPosition,
    ) -> CrdtOp {
        CrdtOp::PlaybackPosition(map_put(
            &mut self.playback_position,
            actor,
            ts,
            user,
            position,
        ))
    }

    // ---- Resolved views.

    /// Resolve the whole state to plain values (LWW winners, sorted
    /// playlist, chat in list order). This is what the UI and the derived
    /// state logic consume, and what convergence tests compare.
    pub fn view(&self) -> StateView {
        StateView {
            playlist: self.playlist_entries(),
            watched: map_view(&self.watched),
            now_playing: resolve_value(&self.now_playing).flatten(),
            seek_authority: resolve_value(&self.seek_authority),
            playback_intent: resolve_value(&self.playback_intent).unwrap_or(PlaybackIntent::Paused),
            series_preference: map_view(&self.series_preference),
            manual_override: map_view(&self.manual_override),
            file_availability: map_view(&self.file_availability),
            anidb_metadata: map_view(&self.anidb_metadata),
            series_relations: map_view(&self.series_relations),
            list_entries: map_view(&self.list_entries),
            list_next_ep: map_view(&self.list_next_ep),
            lookup_requests: self.lookup_requests.read(),
            chat: self
                .chat
                .read::<Vec<&ChatMessage>>()
                .into_iter()
                .cloned()
                .collect(),
            playback_position: map_view(&self.playback_position),
        }
    }
}

impl CrdtState {
    /// The maximum LWW timestamp anywhere in the state: the Lamport
    /// floor after adopting a merge, snapshot, or stored state. (Chat
    /// and the lookup set are excluded — they never compete in a
    /// register.)
    pub fn max_lww_timestamp(&self) -> SharedTimestamp {
        fn map_max<K, V>(map: &LwwMap<K, V>) -> SharedTimestamp
        where
            K: Ord + Clone + Debug,
            V: Ord + Clone + Debug,
        {
            map.iter()
                .filter_map(|entry| entry.val.1.timestamp())
                .max()
                .unwrap_or_default()
        }
        [
            map_max(&self.playlist),
            map_max(&self.watched),
            self.now_playing.timestamp().unwrap_or_default(),
            self.seek_authority.timestamp().unwrap_or_default(),
            self.playback_intent.timestamp().unwrap_or_default(),
            map_max(&self.series_preference),
            map_max(&self.manual_override),
            map_max(&self.file_availability),
            map_max(&self.anidb_metadata),
            map_max(&self.series_relations),
            map_max(&self.list_entries),
            map_max(&self.list_next_ep),
            map_max(&self.playback_position),
        ]
        .into_iter()
        .max()
        .unwrap_or_default()
    }

    /// Hash of the resolved view **excluding playback positions** (they
    /// churn every 100ms and would never match between replicas). Used
    /// by the divergence alarm — see docs/sync-state.md.
    pub fn view_hash(&self) -> [u8; 32] {
        use sha2::Digest;
        let mut view = self.view();
        view.playback_position.clear();
        match crate::wire::encode(&view) {
            Ok(bytes) => sha2::Sha256::digest(&bytes).into(),
            // Encoding a StateView cannot fail (no maps with non-string
            // keys at the serde level, no floats); if it somehow does,
            // return a sentinel that never matches a real hash.
            Err(_) => [0xFF; 32],
        }
    }

    /// Apply an op **received via datagram**, where per-origin FIFO is
    /// not guaranteed. Map ops are applied only if their dot is exactly
    /// the next in sequence for that origin (otherwise applying would
    /// silently mask the gap); register/list/set ops are order-free and
    /// always applied. Returns whether the op was applied — a dropped
    /// op is fine, its reliable copy is on the control stream.
    pub fn apply_if_orderly(&mut self, op: CrdtOp) -> bool {
        /// The op's dot must be `clock[actor] + 1` on this map.
        fn next_in_sequence<K, V>(map: &LwwMap<K, V>, op: &LwwMapOp<K, V>) -> bool
        where
            K: Ord + Clone + Debug,
            V: Ord + Clone + Debug,
        {
            match op {
                crdts::map::Op::Up { dot, .. } => {
                    map.read_ctx().add_clock.get(&dot.actor) + 1 == dot.counter
                }
                // We never send Rm; an out-of-band one is dropped.
                crdts::map::Op::Rm { .. } => false,
            }
        }

        macro_rules! guarded {
            ($field:ident, $op:expr) => {{
                if next_in_sequence(&self.$field, &$op) {
                    self.$field.apply($op);
                    true
                } else {
                    false
                }
            }};
        }

        match op {
            CrdtOp::Playlist(op) => guarded!(playlist, op),
            CrdtOp::Watched(op) => guarded!(watched, op),
            CrdtOp::SeriesPreference(op) => guarded!(series_preference, op),
            CrdtOp::ManualOverride(op) => guarded!(manual_override, op),
            CrdtOp::FileAvailability(op) => guarded!(file_availability, op),
            CrdtOp::AniDbMetadata(op) => guarded!(anidb_metadata, op),
            CrdtOp::SeriesRelations(op) => guarded!(series_relations, op),
            CrdtOp::ListEntry(op) => guarded!(list_entries, op),
            CrdtOp::ListNextEp(op) => guarded!(list_next_ep, op),
            CrdtOp::PlaybackPosition(op) => guarded!(playback_position, op),
            // Order-free types.
            op @ (CrdtOp::NowPlaying(_)
            | CrdtOp::SeekAuthority(_)
            | CrdtOp::PlaybackIntent(_)
            | CrdtOp::LookupRequest(_)
            | CrdtOp::Chat(_)) => {
                self.apply(op);
                true
            }
        }
    }
}

/// The fully resolved, plain-data view of a [`CrdtState`].
/// Serializable so [`CrdtState::view_hash`] can hash it canonically.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateView {
    /// Playlist entries in display order.
    pub playlist: Vec<crate::playlist::PlaylistEntry>,
    /// Group watched flags.
    pub watched: BTreeMap<Ed2kHash, bool>,
    /// The currently playing file.
    pub now_playing: Option<Ed2kHash>,
    /// Current seek authority.
    pub seek_authority: Option<SeekAuthority>,
    /// The play/pause latch (`Paused` when never written).
    pub playback_intent: PlaybackIntent,
    /// Per-user series preferences.
    pub series_preference: BTreeMap<(UserId, AniDbSeriesId), SeriesWatchState>,
    /// Per-user manual overrides.
    pub manual_override: BTreeMap<UserId, Option<ManualState>>,
    /// Per-user file availability.
    pub file_availability: BTreeMap<(UserId, Ed2kHash), FileAvailability>,
    /// File metadata.
    pub anidb_metadata: BTreeMap<Ed2kHash, Option<AniDbMetadata>>,
    /// Franchise relations.
    pub series_relations: BTreeMap<AniDbSeriesId, SeriesRelations>,
    /// The List.
    pub list_entries: BTreeMap<ListEntryId, SeriesListEntry>,
    /// List progress fields.
    pub list_next_ep: BTreeMap<ListEntryId, NextEpState>,
    /// Pending lookup requests.
    pub lookup_requests: BTreeSet<FileHashInfo>,
    /// Chat messages in list order.
    pub chat: Vec<ChatMessage>,
    /// Per-user playback positions.
    pub playback_position: BTreeMap<UserId, PlaybackPosition>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const A1: ActorId = ActorId(1);
    const A2: ActorId = ActorId(2);

    fn ts(t: u64) -> SharedTimestamp {
        SharedTimestamp(t)
    }

    fn hash(i: u8) -> Ed2kHash {
        Ed2kHash([i; 16])
    }

    fn msg(t: u64, who: &str, text: &str) -> ChatMessage {
        ChatMessage {
            timestamp: ts(t),
            sender: UserId::new(who),
            text: text.into(),
        }
    }

    #[test]
    fn chat_preserves_insertion_order() {
        let mut state = CrdtState::new();
        state.append_chat(msg(1, "a", "first"));
        state.append_chat(msg(2, "b", "second"));
        state.append_chat(msg(3, "a", "third"));
        let texts: Vec<String> = state.view().chat.into_iter().map(|m| m.text).collect();
        assert_eq!(texts, vec!["first", "second", "third"]);
    }

    #[test]
    fn chat_ops_are_order_independent_and_idempotent() {
        let mut origin = CrdtState::new();
        let ops: Vec<CrdtOp> = (0..5)
            .map(|i| origin.append_chat(msg(i, "a", &format!("m{i}"))))
            .collect();

        let mut replica = CrdtState::new();
        for op in ops.iter().rev().chain(ops.iter()) {
            replica.apply(op.clone());
        }
        assert_eq!(origin.view().chat, replica.view().chat);
    }

    #[test]
    fn concurrent_register_writes_resolve_by_timestamp() {
        let mut r1 = CrdtState::new();
        let mut r2 = CrdtState::new();
        let op1 = r1.set_now_playing(A1, ts(10), Some(hash(1)));
        let op2 = r2.set_now_playing(A2, ts(20), Some(hash(2)));
        r1.apply(op2);
        r2.apply(op1);
        assert_eq!(r1.view().now_playing, Some(hash(2)));
        assert_eq!(r1.view(), r2.view());
    }

    #[test]
    fn equal_timestamps_tiebreak_on_value() {
        let mut r1 = CrdtState::new();
        let mut r2 = CrdtState::new();
        let op1 = r1.set_now_playing(A1, ts(10), Some(hash(1)));
        let op2 = r2.set_now_playing(A2, ts(10), Some(hash(2)));
        r1.apply(op2);
        r2.apply(op1);
        // max() value wins: hash(2) > hash(1).
        assert_eq!(r1.view().now_playing, Some(hash(2)));
        assert_eq!(r1.view(), r2.view());
    }

    #[test]
    fn older_timestamp_never_wins_even_sequentially() {
        // Pure LWW: a later write with a *lower* timestamp loses, even
        // from the same actor. This is why op generation must issue
        // monotonic timestamps — max(shared_now, last_issued + 1) —
        // a Phase 4 requirement (see docs/sync-state.md).
        let mut state = CrdtState::new();
        state.set_now_playing(A1, ts(100), Some(hash(1)));
        state.set_now_playing(A1, ts(50), Some(hash(2)));
        assert_eq!(state.view().now_playing, Some(hash(1)));
    }

    #[test]
    fn map_ops_are_idempotent() {
        let mut state = CrdtState::new();
        let op = state.set_watched(A1, ts(1), hash(1), true);
        let before = state.clone();
        state.apply(op);
        assert_eq!(state, before);
    }

    #[test]
    fn concurrent_remove_and_readd_resolve_by_lww() {
        // Tombstone semantics: a concurrent remove and re-add resolve by
        // timestamp, identically on every replica.
        let mut r1 = CrdtState::new();
        let mut r2 = CrdtState::new();
        let put = r1.set_playlist_entry(
            A1,
            ts(1),
            hash(1),
            PlaylistFileState {
                position: crdts::Identifier::between(None, None, A1),
                added_by: UserId::new("a"),
                filename: "ep.mkv".into(),
                size_bytes: 1,
                duration_millis: None,
            },
        );
        r2.apply(put);

        // r1 removes (ts 3) while r2 concurrently rewrites (ts 2).
        let rm = r1.remove_playlist_entry(A1, ts(3), hash(1));
        let update = r2.set_playlist_entry(
            A2,
            ts(2),
            hash(1),
            PlaylistFileState {
                position: crdts::Identifier::between(None, None, A2),
                added_by: UserId::new("b"),
                filename: "ep.mkv".into(),
                size_bytes: 2,
                duration_millis: None,
            },
        );
        r1.apply(update);
        r2.apply(rm);

        // The removal's timestamp is later: it wins everywhere.
        assert!(r1.playlist_entries().is_empty());
        assert_eq!(r1.view(), r2.view());
    }

    #[test]
    fn snapshot_round_trips_through_postcard() {
        let mut state = CrdtState::new();
        state.set_now_playing(A1, ts(1), Some(hash(1)));
        state.set_watched(A1, ts(2), hash(1), true);
        state.append_chat(msg(3, "a", "hello"));
        state.request_lookup(FileHashInfo {
            hash: hash(1),
            size: 123,
            filename: "ep1.mkv".into(),
        });

        let snapshot = StateSnapshot {
            epoch: Epoch(7),
            state: state.clone(),
        };
        let bytes = crate::wire::encode(&snapshot).unwrap();
        let decoded: StateSnapshot = crate::wire::decode(&bytes).unwrap();
        assert_eq!(decoded, snapshot);
        assert_eq!(decoded.state.view(), state.view());
    }

    #[test]
    fn merge_is_idempotent() {
        let mut state = CrdtState::new();
        state.set_now_playing(A1, ts(1), Some(hash(1)));
        state.set_manual_override(A1, ts(2), UserId::new("a"), Some(ManualState::Paused));
        let before = state.clone();
        state.merge(before.clone());
        assert_eq!(state, before);
    }
}
