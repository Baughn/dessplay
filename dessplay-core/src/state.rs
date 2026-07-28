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
    FileCatalogEntry, FileHashInfo, ListEntryId, ManualState, MarqueeMessage, NextEpState,
    PlaybackIntent, PlaybackPosition, PlaylistFileState, SeekAuthority, SeriesListEntry,
    SeriesPreference, SeriesRelations, SeriesWatchState, SharedTimestamp, UserId,
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
    /// Per-user, per-series watch preference, keyed on the
    /// [`SeriesListEntry`]'s [`ListEntryId`] rather than [`AniDbSeriesId`]
    /// -- AniDB linking is enrichment only, never a prerequisite for
    /// commitment (design.md, Series Identity).
    pub series_preference: LwwMap<(UserId, ListEntryId), SeriesPreference>,
    /// Per-user manual state override. `Away` is writable by anyone.
    pub manual_override: LwwMap<UserId, Option<ManualState>>,
    /// Per-user, per-file availability.
    pub file_availability: LwwMap<(UserId, Ed2kHash), FileAvailability>,
    /// Server-authoritative file metadata. `None` = not yet looked up.
    pub anidb_metadata: LwwMap<Ed2kHash, Option<AniDbMetadata>>,
    /// Server-authoritative franchise relations graph.
    pub series_relations: LwwMap<AniDbSeriesId, SeriesRelations>,
    /// Server-authoritative file identities, recorded from lookup requests
    /// so a client can add a file it doesn't hold. Survives compaction.
    pub file_catalog: LwwMap<Ed2kHash, FileCatalogEntry>,
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
    /// Per-file one-shot acknowledgements that let the group play past a
    /// committed (Watching) user who is absent: each `(now-playing file,
    /// acknowledged user)` pair suppresses that user's committed-absent
    /// block *for that file only*. Grow-only, cleared at compaction.
    pub acknowledged_absent: GSet<(Ed2kHash, UserId)>,
    /// A transient marquee line every client scrolls on update (today:
    /// AI commentary — design.md, AI Commentary). Cleared at compaction.
    pub marquee: LwwCell<Option<MarqueeMessage>>,
    /// The [`PROTOCOL_VERSION`](crate::net::message::PROTOCOL_VERSION) this
    /// struct's shape matches. Storage snapshots carry an explicit version
    /// tag (the [`SNAPSHOT_MAGIC`] envelope — see
    /// [`CrdtState::encode_snapshot`]), so this field is no longer how
    /// layouts are told apart there; it survives as a length guard for the
    /// one **untagged** legacy layout ([`CrdtStateUntaggedV6`]) and as a
    /// cheap self-description on the wire, where snapshots are embedded
    /// raw. **Must stay the last field** — new fields go before it.
    pub protocol_version: u32,
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
    /// Series preference write (keyed on [`ListEntryId`], not
    /// [`AniDbSeriesId`] -- see [`CrdtState::series_preference`]).
    SeriesPreference(LwwMapOp<(UserId, ListEntryId), SeriesPreference>),
    /// Manual override write.
    ManualOverride(LwwMapOp<UserId, Option<ManualState>>),
    /// File availability write.
    FileAvailability(LwwMapOp<(UserId, Ed2kHash), FileAvailability>),
    /// AniDB metadata write (server).
    AniDbMetadata(LwwMapOp<Ed2kHash, Option<AniDbMetadata>>),
    /// Series relations write (server).
    SeriesRelations(LwwMapOp<AniDbSeriesId, SeriesRelations>),
    /// File catalog write (server).
    FileCatalog(LwwMapOp<Ed2kHash, FileCatalogEntry>),
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
    /// Acknowledged-absent insert (GSet ops are the element itself).
    AcknowledgeAbsent((Ed2kHash, UserId)),
    /// Marquee register write (`None` clears).
    Marquee(LwwRegOp<Option<MarqueeMessage>>),
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
            CrdtOp::FileCatalog(op) => map_ts(op),
            CrdtOp::ListEntry(op) => map_ts(op),
            CrdtOp::ListNextEp(op) => map_ts(op),
            CrdtOp::PlaybackPosition(op) => map_ts(op),
            CrdtOp::NowPlaying(op) => Some(op.timestamp),
            CrdtOp::SeekAuthority(op) => Some(op.timestamp),
            CrdtOp::PlaybackIntent(op) => Some(op.timestamp),
            CrdtOp::Marquee(op) => Some(op.timestamp),
            CrdtOp::LookupRequest(_) | CrdtOp::Chat(_) | CrdtOp::AcknowledgeAbsent(_) => None,
        }
    }

    /// The variant's name, for logging (Debug-formatting whole ops is
    /// too noisy even at trace level).
    pub fn variant_name(&self) -> &'static str {
        match self {
            CrdtOp::Playlist(_) => "Playlist",
            CrdtOp::Watched(_) => "Watched",
            CrdtOp::NowPlaying(_) => "NowPlaying",
            CrdtOp::SeekAuthority(_) => "SeekAuthority",
            CrdtOp::PlaybackIntent(_) => "PlaybackIntent",
            CrdtOp::SeriesPreference(_) => "SeriesPreference",
            CrdtOp::ManualOverride(_) => "ManualOverride",
            CrdtOp::FileAvailability(_) => "FileAvailability",
            CrdtOp::AniDbMetadata(_) => "AniDbMetadata",
            CrdtOp::SeriesRelations(_) => "SeriesRelations",
            CrdtOp::FileCatalog(_) => "FileCatalog",
            CrdtOp::ListEntry(_) => "ListEntry",
            CrdtOp::ListNextEp(_) => "ListNextEp",
            CrdtOp::LookupRequest(_) => "LookupRequest",
            CrdtOp::Chat(_) => "Chat",
            CrdtOp::PlaybackPosition(_) => "PlaybackPosition",
            CrdtOp::AcknowledgeAbsent(_) => "AcknowledgeAbsent",
            CrdtOp::Marquee(_) => "Marquee",
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

/// Storage-envelope magic prefixing every tagged snapshot blob. The
/// first byte is 0xFF: an untagged postcard [`CrdtState`] begins with
/// the playlist map's vclock length varint, and 0xFF there would claim
/// a continuation-varint clock size no real state can reach — so the
/// magic can never collide with a legacy blob (pinned by
/// `untagged_v6_blob_decodes_via_the_legacy_fallback`).
pub const SNAPSHOT_MAGIC: [u8; 4] = [0xFF, b'D', b'S', b'S'];

/// The **untagged** on-disk layout of [`CrdtState`] as written by
/// protocol-v6 builds, before storage snapshots gained the
/// [`SNAPSHOT_MAGIC`] envelope. Field-for-field the v6 shape, frozen so
/// later [`CrdtState`] changes cannot silently alter what this decodes.
/// This is the one legacy fallback [`CrdtState::decode_snapshot`]
/// keeps: every database deployed at the envelope change (clients and
/// the authoritative server) held exactly this layout. The older
/// V1..V5 fallback chain — five frozen structs plus upgrade helpers,
/// grown one per shape change because untagged blobs could only be
/// told apart by trial decode — was deleted along with the trial
/// decoding; docs/sync-state.md keeps the retrospective.
#[derive(Deserialize)]
#[cfg_attr(any(test, feature = "test-support"), derive(Default, Serialize))]
struct CrdtStateUntaggedV6 {
    playlist: LwwMap<Ed2kHash, Option<PlaylistFileState>>,
    watched: LwwMap<Ed2kHash, bool>,
    now_playing: LwwCell<Option<Ed2kHash>>,
    seek_authority: LwwCell<SeekAuthority>,
    playback_intent: LwwCell<PlaybackIntent>,
    series_preference: LwwMap<(UserId, ListEntryId), SeriesPreference>,
    manual_override: LwwMap<UserId, Option<ManualState>>,
    file_availability: LwwMap<(UserId, Ed2kHash), FileAvailability>,
    anidb_metadata: LwwMap<Ed2kHash, Option<AniDbMetadata>>,
    series_relations: LwwMap<AniDbSeriesId, SeriesRelations>,
    file_catalog: LwwMap<Ed2kHash, FileCatalogEntry>,
    list_entries: LwwMap<ListEntryId, SeriesListEntry>,
    list_next_ep: LwwMap<ListEntryId, NextEpState>,
    lookup_requests: GSet<FileHashInfo>,
    chat: GList<ChatMessage>,
    playback_position: LwwMap<UserId, PlaybackPosition>,
    acknowledged_absent: GSet<(Ed2kHash, UserId)>,
    /// Read for layout only (postcard is positional, so the name is
    /// free); the stored value (6) is discarded on upgrade.
    _protocol_version: u32,
}

impl From<CrdtStateUntaggedV6> for CrdtState {
    fn from(v6: CrdtStateUntaggedV6) -> Self {
        CrdtState {
            playlist: v6.playlist,
            watched: v6.watched,
            now_playing: v6.now_playing,
            seek_authority: v6.seek_authority,
            playback_intent: v6.playback_intent,
            series_preference: v6.series_preference,
            manual_override: v6.manual_override,
            file_availability: v6.file_availability,
            anidb_metadata: v6.anidb_metadata,
            series_relations: v6.series_relations,
            file_catalog: v6.file_catalog,
            list_entries: v6.list_entries,
            list_next_ep: v6.list_next_ep,
            lookup_requests: v6.lookup_requests,
            chat: v6.chat,
            playback_position: v6.playback_position,
            acknowledged_absent: v6.acknowledged_absent,
            marquee: LwwCell::default(),
            protocol_version: crate::net::message::PROTOCOL_VERSION,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl CrdtState {
    /// Encode this state in the **untagged legacy v6** layout (the
    /// pre-envelope on-disk shape), for fabricating faithful migration
    /// fixtures — including from other crates' tests, which cannot reach
    /// the private [`CrdtStateUntaggedV6`]. Drops any post-v6 fields.
    pub fn encode_untagged_v6_for_tests(&self) -> Result<Vec<u8>, crate::wire::WireError> {
        let state = self.clone();
        crate::wire::encode(&CrdtStateUntaggedV6 {
            playlist: state.playlist,
            watched: state.watched,
            now_playing: state.now_playing,
            seek_authority: state.seek_authority,
            playback_intent: state.playback_intent,
            series_preference: state.series_preference,
            manual_override: state.manual_override,
            file_availability: state.file_availability,
            anidb_metadata: state.anidb_metadata,
            series_relations: state.series_relations,
            file_catalog: state.file_catalog,
            list_entries: state.list_entries,
            list_next_ep: state.list_next_ep,
            lookup_requests: state.lookup_requests,
            chat: state.chat,
            playback_position: state.playback_position,
            acknowledged_absent: state.acknowledged_absent,
            _protocol_version: 6,
        })
    }
}

impl CrdtState {
    /// Encode this state for **storage**: [`SNAPSHOT_MAGIC`] ++
    /// [`PROTOCOL_VERSION`](crate::net::message::PROTOCOL_VERSION)
    /// (u32 LE) ++ postcard(state). Wire messages embed the raw postcard
    /// shape instead — cross-version wire compatibility is owned entirely
    /// by the `Auth` handshake's version gate, while storage blobs
    /// outlive deployments and need the self-describing tag.
    pub fn encode_snapshot(&self) -> Result<Vec<u8>, crate::wire::WireError> {
        let body = crate::wire::encode(self)?;
        let mut blob = Vec::with_capacity(SNAPSHOT_MAGIC.len() + 4 + body.len());
        blob.extend_from_slice(&SNAPSHOT_MAGIC);
        blob.extend_from_slice(&crate::net::message::PROTOCOL_VERSION.to_le_bytes());
        blob.extend_from_slice(&body);
        Ok(blob)
    }

    /// Decode a persisted snapshot blob. A tagged blob (the
    /// [`SNAPSHOT_MAGIC`] envelope) names its exact layout: the version
    /// must equal the running binary's, or this errors rather than
    /// guessing — the refuse-to-start posture; a deliberate migration
    /// adds an explicit decode arm for the old version instead. An
    /// untagged blob is the one legacy layout ([`CrdtStateUntaggedV6`])
    /// and is migrated forward.
    pub fn decode_snapshot(blob: &[u8]) -> Result<CrdtState, crate::wire::WireError> {
        Ok(Self::decode_snapshot_flagged(blob)?.0)
    }

    /// Older tagged versions whose persisted [`CrdtState`] layout is
    /// **identical** to the current one, accepted as a deliberate
    /// migration decision (the refuse-to-guess policy's explicit decode
    /// arm): v7 → v9 changed only wire messages — the DSCP transfer-
    /// connection split (v8) and per-transfer data streams (v9) — never
    /// a replicated value type or `CrdtState` itself. Every entry here
    /// asserts "I checked the diff; the postcard body did not move."
    const LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS: [u32; 2] = [7, 8];

    /// [`decode_snapshot`](Self::decode_snapshot), also reporting whether
    /// a **migration** was used (`true` = the blob was written by an
    /// older build: the untagged v6 fallback, or a layout-compatible
    /// older tag). A caller that will persist the migrated result over
    /// the original — the rendezvous server — uses the flag to back up
    /// the old database first, so a subtly-wrong migration is
    /// recoverable.
    pub fn decode_snapshot_flagged(
        blob: &[u8],
    ) -> Result<(CrdtState, bool), crate::wire::WireError> {
        if let Some(tagged) = blob.strip_prefix(&SNAPSHOT_MAGIC) {
            let Some((version_bytes, body)) = tagged.split_first_chunk::<4>() else {
                return Err(crate::wire::WireError::DeserializeUnexpectedEnd);
            };
            let version = u32::from_le_bytes(*version_bytes);
            if version == crate::net::message::PROTOCOL_VERSION {
                return Ok((crate::wire::decode::<CrdtState>(body)?, false));
            }
            if Self::LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS.contains(&version) {
                tracing::info!(
                    stored = version,
                    current = crate::net::message::PROTOCOL_VERSION,
                    "snapshot from a layout-compatible older protocol; migrating"
                );
                let mut state = crate::wire::decode::<CrdtState>(body)?;
                state.protocol_version = crate::net::message::PROTOCOL_VERSION;
                return Ok((state, true));
            }
            tracing::error!(
                stored = version,
                current = crate::net::message::PROTOCOL_VERSION,
                "snapshot blob is tagged with a different protocol version; \
                 refusing to guess at its layout"
            );
            return Err(crate::wire::WireError::DeserializeBadEncoding);
        }
        crate::wire::decode::<CrdtStateUntaggedV6>(blob)
            .map(|legacy| (CrdtState::from(legacy), true))
    }
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
        Self {
            protocol_version: crate::net::message::PROTOCOL_VERSION,
            ..Self::default()
        }
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
            CrdtOp::FileCatalog(op) => self.file_catalog.apply(op),
            CrdtOp::ListEntry(op) => self.list_entries.apply(op),
            CrdtOp::ListNextEp(op) => self.list_next_ep.apply(op),
            CrdtOp::LookupRequest(info) => self.lookup_requests.apply(info),
            CrdtOp::Chat(op) => self.chat.apply(op),
            CrdtOp::PlaybackPosition(op) => self.playback_position.apply(op),
            CrdtOp::AcknowledgeAbsent(key) => self.acknowledged_absent.apply(key),
            CrdtOp::Marquee(op) => self.marquee.apply(op),
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
        self.file_catalog.merge(other.file_catalog);
        self.list_entries.merge(other.list_entries);
        self.list_next_ep.merge(other.list_next_ep);
        self.lookup_requests.merge(other.lookup_requests);
        self.chat.merge(other.chat);
        self.playback_position.merge(other.playback_position);
        self.acknowledged_absent.merge(other.acknowledged_absent);
        self.marquee.merge(other.marquee);
        // Not a CRDT type -- both sides are always the current binary's
        // PROTOCOL_VERSION in practice (the connect-time gate refuses a
        // mismatched peer), so which side wins is moot; `max` is a
        // deterministic, order-independent pick.
        self.protocol_version = self.protocol_version.max(other.protocol_version);
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

    /// Set a user's watch preference for a series. `set_by` names the
    /// writer when it isn't `user` themself (`None` for every self-directed
    /// write and system auto-write — see [`SeriesPreference`]).
    pub fn set_series_preference(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        user: UserId,
        entry: ListEntryId,
        pref: SeriesWatchState,
        set_by: Option<UserId>,
    ) -> CrdtOp {
        CrdtOp::SeriesPreference(map_put(
            &mut self.series_preference,
            actor,
            ts,
            (user, entry),
            SeriesPreference {
                state: pref,
                set_by,
            },
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

    /// Write a file's catalog identity. Server-only by convention.
    pub fn set_file_catalog(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        hash: Ed2kHash,
        entry: FileCatalogEntry,
    ) -> CrdtOp {
        CrdtOp::FileCatalog(map_put(&mut self.file_catalog, actor, ts, hash, entry))
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

    /// Record a per-file acknowledgement that the group accepts playing
    /// `file` without the committed-but-absent `user` (see the field
    /// docs). Any peer may write it; it gates only against the current
    /// now-playing file and is cleared at compaction.
    pub fn acknowledge_absent(&mut self, file: Ed2kHash, user: UserId) -> CrdtOp {
        let key = (file, user);
        self.acknowledged_absent.apply(key.clone());
        CrdtOp::AcknowledgeAbsent(key)
    }

    /// Write (or, with `None`, clear) the marquee register. (See
    /// [`Self::set_now_playing`] on the unused `actor`.)
    pub fn set_marquee(
        &mut self,
        actor: ActorId,
        ts: SharedTimestamp,
        message: Option<MarqueeMessage>,
    ) -> CrdtOp {
        let _ = actor;
        CrdtOp::Marquee(reg_put(&mut self.marquee, ts, message))
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
            file_catalog: map_view(&self.file_catalog),
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
            acknowledged_absent: self.acknowledged_absent.read(),
            marquee: self
                .marquee
                .read()
                .and_then(|lww| lww.value.clone().map(|m| (lww.timestamp, m))),
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
            self.marquee.timestamp().unwrap_or_default(),
            map_max(&self.series_preference),
            map_max(&self.manual_override),
            map_max(&self.file_availability),
            map_max(&self.anidb_metadata),
            map_max(&self.series_relations),
            map_max(&self.file_catalog),
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
            CrdtOp::FileCatalog(op) => guarded!(file_catalog, op),
            CrdtOp::ListEntry(op) => guarded!(list_entries, op),
            CrdtOp::ListNextEp(op) => guarded!(list_next_ep, op),
            CrdtOp::PlaybackPosition(op) => guarded!(playback_position, op),
            // Order-free types.
            op @ (CrdtOp::NowPlaying(_)
            | CrdtOp::SeekAuthority(_)
            | CrdtOp::PlaybackIntent(_)
            | CrdtOp::Marquee(_)
            | CrdtOp::LookupRequest(_)
            | CrdtOp::Chat(_)
            | CrdtOp::AcknowledgeAbsent(_)) => {
                self.apply(op);
                true
            }
        }
    }

    /// Apply a map op received on the **reliable** control stream, returning
    /// whether it advanced this origin's dot clock (i.e. was genuinely new).
    ///
    /// Unlike the datagram path this applies even out-of-sequence — reliable
    /// delivery is the gap-fill fallback — but a copy whose dot we have
    /// *already* seen (its eager datagram twin arrived first and applied) is
    /// an idempotent no-op and must **not** rebroadcast a second time. This
    /// mirrors the change-detection the datagram arm gets from
    /// [`Self::apply_if_orderly`], closing the datagram-first double-broadcast.
    fn apply_map_reliable(&mut self, op: CrdtOp) -> bool {
        /// The op carries a dot beyond what we've applied for its origin.
        fn advances<K, V>(map: &LwwMap<K, V>, op: &LwwMapOp<K, V>) -> bool
        where
            K: Ord + Clone + Debug,
            V: Ord + Clone + Debug,
        {
            match op {
                crdts::map::Op::Up { dot, .. } => {
                    map.read_ctx().add_clock.get(&dot.actor) < dot.counter
                }
                // We never send Rm; an out-of-band one is dropped.
                crdts::map::Op::Rm { .. } => false,
            }
        }

        macro_rules! reliable {
            ($field:ident, $op:expr) => {{
                let advanced = advances(&self.$field, &$op);
                self.$field.apply($op);
                advanced
            }};
        }

        match op {
            CrdtOp::Playlist(op) => reliable!(playlist, op),
            CrdtOp::Watched(op) => reliable!(watched, op),
            CrdtOp::SeriesPreference(op) => reliable!(series_preference, op),
            CrdtOp::ManualOverride(op) => reliable!(manual_override, op),
            CrdtOp::FileAvailability(op) => reliable!(file_availability, op),
            CrdtOp::AniDbMetadata(op) => reliable!(anidb_metadata, op),
            CrdtOp::SeriesRelations(op) => reliable!(series_relations, op),
            CrdtOp::FileCatalog(op) => reliable!(file_catalog, op),
            CrdtOp::ListEntry(op) => reliable!(list_entries, op),
            CrdtOp::ListNextEp(op) => reliable!(list_next_ep, op),
            CrdtOp::PlaybackPosition(op) => reliable!(playback_position, op),
            // Order-free types are handled by the caller before the map arm;
            // reached only defensively, they apply and rebroadcast.
            other => {
                self.apply(other);
                true
            }
        }
    }

    /// Apply an op the server received from a client, returning whether it
    /// should **re-fan-out** this copy to the other peers.
    ///
    /// Every ordinary op is sent *eager* — once on the reliable control
    /// stream and once as a datagram — so the server receives two copies and
    /// must broadcast the op only once (sync-state.md, Operation Broadcast:
    /// "the server deduplicates, applies, and broadcasts").
    ///
    /// - **Map ops** carry per-origin dot sequencing. On the datagram fast
    ///   path an out-of-sequence op is *dropped* (not applied ahead of an
    ///   undelivered earlier dot, which would silently mask the gap — its
    ///   reliable copy fills it); only an in-sequence (new) op rebroadcasts.
    ///   On the reliable path they apply unconditionally (the gap-fill
    ///   fallback) and rebroadcast, advancing every peer's clock.
    /// - **Order-free ops** (registers, the two GSets, chat) have no
    ///   sequencing and apply unconditionally either way, but rebroadcast
    ///   only when they *change* the resolved state — so the second
    ///   (no-op) copy of an eager op isn't fanned out again.
    pub fn apply_for_broadcast(&mut self, op: CrdtOp, via_datagram: bool) -> bool {
        match op {
            // Order-free: change-detected, identically on both transports.
            CrdtOp::NowPlaying(op) => register_changed(&mut self.now_playing, op),
            CrdtOp::SeekAuthority(op) => register_changed(&mut self.seek_authority, op),
            CrdtOp::PlaybackIntent(op) => register_changed(&mut self.playback_intent, op),
            CrdtOp::Marquee(op) => register_changed(&mut self.marquee, op),
            CrdtOp::LookupRequest(info) => gset_changed(&mut self.lookup_requests, info),
            CrdtOp::AcknowledgeAbsent(key) => gset_changed(&mut self.acknowledged_absent, key),
            CrdtOp::Chat(op) => glist_changed(&mut self.chat, op),
            // Map ops: datagram drops gaps (`apply_if_orderly`); reliable
            // applies unconditionally but gap-fills (`apply_map_reliable`).
            // Both change-detect on the origin dot clock, so an eager op's
            // second copy (whichever transport lost the race) is a no-op and
            // is not fanned out twice.
            map_op if via_datagram => self.apply_if_orderly(map_op),
            map_op => self.apply_map_reliable(map_op),
        }
    }
}

/// Apply a register op, reporting whether it changed the resolved winner.
/// Compares the full `(timestamp, value)` winner, not just the value: an
/// LWW write that only advances the timestamp (same value) must still
/// propagate, or a later concurrent write could resolve differently on a
/// replica that never saw it.
fn register_changed<V: Ord + Clone>(cell: &mut LwwCell<V>, op: Lww<V>) -> bool {
    let before = cell.read().cloned();
    cell.apply(op);
    cell.read() != before.as_ref()
}

/// Apply a GSet insert, reporting whether the element was new.
fn gset_changed<T: Ord>(set: &mut GSet<T>, element: T) -> bool {
    if set.contains(&element) {
        false
    } else {
        set.apply(element);
        true
    }
}

/// Apply a GList insert, reporting whether it added a new entry. A duplicate
/// identifier is idempotent (BTreeSet dedup) and leaves the length unchanged.
fn glist_changed<T: Ord + Clone>(list: &mut GList<T>, op: glist::Op<T>) -> bool {
    let before = list.len();
    list.apply(op);
    list.len() != before
}

/// The fully resolved, plain-data view of a [`CrdtState`].
/// Serializable so [`CrdtState::view_hash`] can hash it canonically.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Per-user series preferences, keyed on the List entry.
    pub series_preference: BTreeMap<(UserId, ListEntryId), SeriesPreference>,
    /// Per-user manual overrides.
    pub manual_override: BTreeMap<UserId, Option<ManualState>>,
    /// Per-user file availability.
    pub file_availability: BTreeMap<(UserId, Ed2kHash), FileAvailability>,
    /// File metadata.
    pub anidb_metadata: BTreeMap<Ed2kHash, Option<AniDbMetadata>>,
    /// Franchise relations.
    pub series_relations: BTreeMap<AniDbSeriesId, SeriesRelations>,
    /// File identities for the collective library (server-written).
    pub file_catalog: BTreeMap<Ed2kHash, FileCatalogEntry>,
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
    /// Per-file acknowledgements of committed-but-absent users.
    pub acknowledged_absent: BTreeSet<(Ed2kHash, UserId)>,
    /// The marquee line with its LWW stamp — the stamp keys the UI's
    /// scroll animation (a rewrite of the same text still replays).
    pub marquee: Option<(SharedTimestamp, MarqueeMessage)>,
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

    /// The one surviving legacy fallback: an **untagged** protocol-v6
    /// blob (the layout every deployed database held when storage
    /// snapshots gained the [`SNAPSHOT_MAGIC`] envelope) must decode
    /// flagged and migrate forward intact. Also pins the envelope's
    /// discriminator: no legacy blob can begin with the magic's 0xFF.
    #[test]
    fn untagged_v6_blob_decodes_via_the_legacy_fallback() {
        let mut legacy = CrdtStateUntaggedV6::default();
        reg_put(&mut legacy.now_playing, ts(1), Some(hash(1)));
        map_put(&mut legacy.watched, A1, ts(2), hash(1), true);
        legacy
            .acknowledged_absent
            .apply((hash(2), UserId::new("kim")));
        legacy._protocol_version = 6;
        let blob = crate::wire::encode(&legacy).unwrap();
        assert_ne!(
            blob[0], SNAPSHOT_MAGIC[0],
            "an untagged blob must never collide with the envelope magic"
        );

        let (decoded, migrated) =
            CrdtState::decode_snapshot_flagged(&blob).expect("legacy v6 blob must decode");
        assert!(migrated, "an untagged blob must report the fallback");
        assert_eq!(decoded.view().now_playing, Some(hash(1)));
        assert_eq!(decoded.view().watched.get(&hash(1)), Some(&true));
        assert!(
            decoded
                .view()
                .acknowledged_absent
                .contains(&(hash(2), UserId::new("kim")))
        );
        assert_eq!(decoded.view().marquee, None, "post-v6 fields come up empty");
        assert_eq!(
            decoded.protocol_version,
            crate::net::message::PROTOCOL_VERSION
        );
    }

    /// Regression (2026-07-28, the tsugumi deploy): the server refused
    /// to start on its own authoritative snapshot after the v7 → v9
    /// protocol bump, because the bump changed only *wire* messages —
    /// the persisted `CrdtState` layout is identical — but no explicit
    /// decode arm said so. A layout-compatible older tag must decode,
    /// flagged as a migration (so the server backs up the database
    /// before persisting the re-tagged blob); a tag outside the
    /// compatible set must still be refused, not guessed at.
    #[test]
    fn layout_compatible_tagged_versions_migrate_and_others_refuse() {
        let mut state = CrdtState::new();
        reg_put(&mut state.now_playing, ts(1), Some(hash(1)));
        map_put(&mut state.watched, A1, ts(2), hash(1), true);
        let blob = state.encode_snapshot().unwrap();
        let retag = |version: u32| {
            let mut blob = blob.clone();
            blob[SNAPSHOT_MAGIC.len()..SNAPSHOT_MAGIC.len() + 4]
                .copy_from_slice(&version.to_le_bytes());
            blob
        };

        for old in CrdtState::LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS {
            let (decoded, migrated) = CrdtState::decode_snapshot_flagged(&retag(old))
                .unwrap_or_else(|e| panic!("v{old}-tagged blob must decode: {e}"));
            assert!(migrated, "a v{old} tag must report the migration");
            assert_eq!(decoded.view().now_playing, Some(hash(1)));
            assert_eq!(decoded.view().watched.get(&hash(1)), Some(&true));
            assert_eq!(
                decoded.protocol_version,
                crate::net::message::PROTOCOL_VERSION,
                "the migrated state re-tags itself"
            );
        }

        // The current tag is not a migration.
        let (_, migrated) = CrdtState::decode_snapshot_flagged(&blob).unwrap();
        assert!(!migrated);

        // Anything outside the compatible set stays refused.
        for unknown in [3, 6, crate::net::message::PROTOCOL_VERSION + 1] {
            assert!(
                CrdtState::decode_snapshot_flagged(&retag(unknown)).is_err(),
                "a v{unknown}-tagged blob must be refused, not guessed at"
            );
        }
    }

    /// A marquee-only state must report its stamp from
    /// `max_lww_timestamp` — the Lamport floor a restart re-seeds from.
    /// Missing that arm compiles fine and silently lets a restarted
    /// client re-issue spent stamps (and lose LWW races it should win).
    #[test]
    fn marquee_write_raises_the_lamport_floor() {
        let mut state = CrdtState::new();
        let op = state.set_marquee(
            A1,
            ts(42),
            Some(MarqueeMessage {
                text: "<Amu> Whaaaat?".into(),
                set_by: Some(UserId::new("baughn")),
            }),
        );
        assert_eq!(state.max_lww_timestamp(), ts(42));
        assert_eq!(op.lww_timestamp(), Some(ts(42)));
    }

    #[test]
    fn marquee_round_trips_through_view_and_clears() {
        let mut state = CrdtState::new();
        assert_eq!(state.view().marquee, None);
        let msg = MarqueeMessage {
            text: "hi".into(),
            set_by: None,
        };
        state.set_marquee(A1, ts(5), Some(msg.clone()));
        assert_eq!(state.view().marquee, Some((ts(5), msg)));
        // A clear is a tombstone write, not an absence: it wins by LWW.
        state.set_marquee(A2, ts(6), None);
        assert_eq!(state.view().marquee, None);
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

    /// Regression: an eager *map* op (a reliable control copy AND a datagram
    /// copy of the same op) must rebroadcast exactly once regardless of which
    /// transport the server processes first. The reliable arm used to return
    /// `true` unconditionally, so a datagram-first map op applied and
    /// rebroadcast, then its reliable twin re-applied as a no-op and
    /// rebroadcast a second time — doubled relay egress for the whole map-op
    /// class.
    #[test]
    fn eager_map_op_rebroadcasts_once_on_either_transport_first() {
        let mut client = CrdtState::new();
        let op = client.set_file_availability(
            A1,
            ts(1),
            UserId::new("kim"),
            hash(1),
            FileAvailability::Ready,
        );

        // Datagram copy first: new → rebroadcast; the reliable twin is an
        // idempotent no-op → must NOT rebroadcast again.
        let mut server = CrdtState::new();
        assert!(
            server.apply_for_broadcast(op.clone(), true),
            "the datagram copy is new"
        );
        assert!(
            !server.apply_for_broadcast(op.clone(), false),
            "the reliable twin of an already-applied map op must not rebroadcast"
        );

        // Symmetric — reliable copy first, then the datagram twin is a no-op.
        let mut server = CrdtState::new();
        assert!(
            server.apply_for_broadcast(op.clone(), false),
            "the reliable copy is new"
        );
        assert!(
            !server.apply_for_broadcast(op.clone(), true),
            "the datagram twin of an already-applied map op must not rebroadcast"
        );
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
    fn file_catalog_resolves_by_lww_and_views() {
        use crate::types::FileCatalogEntry;
        let entry = |name: &str, size: u64| FileCatalogEntry {
            filename: name.into(),
            size_bytes: size,
            duration_millis: None,
        };
        let mut r1 = CrdtState::new();
        let mut r2 = CrdtState::new();
        let op1 = r1.set_file_catalog(A1, ts(10), hash(1), entry("old.mkv", 1));
        let op2 = r2.set_file_catalog(A2, ts(20), hash(1), entry("new.mkv", 2));
        r1.apply(op2);
        r2.apply(op1);
        // Later timestamp wins, on both replicas.
        assert_eq!(
            r1.view().file_catalog.get(&hash(1)),
            Some(&entry("new.mkv", 2))
        );
        assert_eq!(r1.view(), r2.view());
    }
}
