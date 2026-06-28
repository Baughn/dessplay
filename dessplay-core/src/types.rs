//! Core identifier and domain types shared across all DessPlay components.
//!
//! Everything here is plain data: serializable, orderable, and free of any
//! CRDT machinery. The `Ord` impls matter — LWW conflict resolution
//! tiebreaks on value (see [`crate::lww::Lww`]), so every type stored in a
//! register must have a deterministic total order.

use std::collections::BTreeSet;
use std::fmt;

use crdts::Identifier;
use serde::{Deserialize, Serialize};

/// The ed2k root hash of a file's contents — DessPlay's `FileId`.
///
/// Computed with the eMule/AniDB ("red") variant: files whose size is an
/// exact multiple of the block size include a trailing empty-block hash.
/// See [`crate::hash`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Ed2kHash(pub [u8; 16]);

impl fmt::Display for Ed2kHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Ed2kHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ed2kHash({self})")
    }
}

/// A user's self-chosen nickname. There is no cryptographic identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct UserId(pub String);

impl UserId {
    /// Construct from anything string-like.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UserId({})", self.0)
    }
}

/// Unique identifier for a participant in the CRDT system.
///
/// Each client has one; the server uses the well-known [`ActorId::SERVER`]
/// for authoritative actions (EOF transitions, seek authority on file
/// change, AniDB metadata writes).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ActorId(pub u128);

impl ActorId {
    /// The well-known server actor.
    pub const SERVER: ActorId = ActorId(0);

    /// Derive a fresh **session-scoped** ActorId from a username and a
    /// caller-supplied random nonce. Never collides with
    /// [`ActorId::SERVER`].
    ///
    /// Actors are per-session by design: Map ops carry per-actor
    /// sequence numbers, and a client restarting from a stale snapshot
    /// would otherwise re-allocate numbers its previous incarnation
    /// already spent (double-spent dots — state corruption). A fresh
    /// actor per session makes that structurally impossible. Map clocks
    /// accumulate one entry per session; compaction collapses them by
    /// rebuilding state under the server actor (see sync-state.md).
    pub fn session(name: &str, nonce: u128) -> Self {
        use digest::Digest;
        let mut hasher = md4::Md4::new();
        hasher.update(name.as_bytes());
        hasher.update(nonce.to_le_bytes());
        let id = u128::from_le_bytes(hasher.finalize().into());
        // 0 is reserved for the server; remap the (astronomically
        // unlikely) collision.
        Self(id.max(1))
    }

    /// True for the well-known server actor.
    pub fn is_server(self) -> bool {
        self == Self::SERVER
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_server() {
            f.write_str("SERVER")
        } else {
            write!(f, "{:032x}", self.0)
        }
    }
}

impl fmt::Debug for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_server() {
            write!(f, "ActorId(SERVER)")
        } else {
            write!(f, "ActorId({:032x})", self.0)
        }
    }
}

/// A timestamp on the shared clock established with the rendezvous server,
/// in milliseconds since the Unix epoch.
///
/// All LWW conflict resolution orders by these, so they must come from the
/// shared clock — never from the local system clock directly.
#[derive(
    Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize,
)]
pub struct SharedTimestamp(pub u64);

impl SharedTimestamp {
    /// Construct from milliseconds since the Unix epoch.
    pub fn from_millis(millis: u64) -> Self {
        Self(millis)
    }

    /// Milliseconds since the Unix epoch.
    pub fn as_millis(self) -> u64 {
        self.0
    }
}

/// An AniDB anime id (`aid`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct AniDbSeriesId(pub u32);

/// Identifier for an entry in The List: a random 128-bit id generated at
/// entry creation or import. The caller supplies the randomness.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct ListEntryId(pub u128);

impl ListEntryId {
    /// Construct from 16 caller-provided random bytes.
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(u128::from_le_bytes(bytes))
    }
}

/// State-compaction generation counter. Incremented by the server on each
/// compaction; clients with a stale epoch replace their state wholesale.
#[derive(
    Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize,
)]
pub struct Epoch(pub u64);

impl Epoch {
    /// The next epoch.
    pub fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

/// A playlist entry's replicated state. Stored under the file's
/// [`Ed2kHash`] in the playlist map; the watched flag deliberately lives in
/// a separate map (see docs/sync-state.md).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct PlaylistFileState {
    /// Dense ordering position. Rebalanced by the server at compaction.
    pub position: Identifier<ActorId>,
    /// Who added this file.
    pub added_by: UserId,
    /// Original filename, for display and local matching.
    pub filename: String,
    /// Filled by the adder; downloaders need it for chunk counts.
    pub size_bytes: u64,
    /// Filled by the adder; drives the bitrate unpause rule and watched
    /// thresholds for files still downloading. `None` if unknown.
    pub duration_millis: Option<u64>,
}

/// Per-user, per-series watch preference: a user's commitment to a series.
///
/// Three states, with an *absent* map entry meaning [`Maybe`](Self::Maybe)
/// — the common default. `Maybe` is a **trailing** variant on purpose:
/// serde/postcard number variants by declaration order, so the two older
/// discriminants stay byte-identical and snapshots written before `Maybe`
/// existed still deserialize (see docs/sync-state.md, Series Preference).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum SeriesWatchState {
    /// Committed: the group waits for this user on this series **even when
    /// they are absent** (Lost/Departed/quit). The only state that gates
    /// across absence.
    Watching,
    /// The user skips this series and never gates playback on it, present
    /// or absent; it is also not auto-downloaded.
    NotWatching,
    /// The default (also the value of an absent entry): opportunistic.
    /// Gates only while the user is *present* and not ready-to-play; an
    /// absent Maybe user does not block. Stored explicitly so a user can
    /// cycle back to it from Watching/NotWatching (the CRDT has no key
    /// removal — see docs/sync-state.md).
    Maybe,
}

/// Who is currently the playback-position authority. A user identity
/// rather than an `ActorId`: actors are session-scoped, so a raw actor
/// could not be mapped back to a user across reconnects.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum SeekAuthority {
    /// The server holds authority (file changes, authority departure).
    Server,
    /// The named user holds authority (they seeked last).
    User(UserId),
}

/// The group's shared play/pause latch. Whether video *actually* plays
/// is derived (see [`crate::derive`]): intent must be `Playing`, every
/// present interactive user must permit playback, and nobody may be
/// Lost. The register exists because gating alone cannot express "stays
/// paused after the blocker departs" — without it, playback would
/// silently auto-resume the moment a paused or lost user drops out of
/// the gating set.
///
/// Users write it on play/pause; the server forces `Paused` on Lost, on
/// graceful quit, on departure, and when EOF advances now-playing.
#[derive(
    Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize,
)]
pub enum PlaybackIntent {
    /// Nobody has pressed play (or something forced a pause). The
    /// fresh-state default.
    #[default]
    Paused,
    /// Someone pressed play; video runs if gating permits.
    Playing,
}

/// A user's manual state override. `None` in the register means no
/// override.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum ManualState {
    /// The user paused (stepped away); blocks playback.
    Paused,
    /// Marked away by another user; does not block playback.
    Away {
        /// Who set it, for display ("away, set by Baughn").
        set_by: UserId,
    },
}

/// A user's ability to play a given file.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum FileAvailability {
    /// Hash matches, file loaded, can unpause at will.
    Ready,
    /// File absent or hash mismatched; blocks playback.
    Missing,
    /// Actively downloading from peers.
    Downloading {
        /// Progress in basis points (0–10000); integer to keep `Eq`/`Ord`.
        progress_bps: u16,
    },
}

/// Where a file's metadata came from.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum MetadataSource {
    /// A successful AniDB lookup.
    AniDb,
    /// AniDB didn't know the file; series name parsed from the filename.
    FilenameDerived,
}

/// Server-written metadata for a file. Always has a series name; the id and
/// episode number are only present for real AniDB hits.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct AniDbMetadata {
    /// Lookup vs filename fallback.
    pub source: MetadataSource,
    /// Always present.
    pub series_name: String,
    /// `None` if filename-derived.
    pub series_id: Option<AniDbSeriesId>,
    /// `None` if unknown. String because AniDB uses "S1", "C1", etc.
    pub episode_number: Option<String>,
}

/// Server-written file identity, recorded from a lookup request so a client
/// that has never held the file can still construct a playlist entry for it
/// (and download it). Distinct from [`AniDbMetadata`], which is AniDB's
/// classification of the file; this is just what the file *is*.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct FileCatalogEntry {
    /// Original filename, from the lookup request.
    pub filename: String,
    /// File size in bytes, from the lookup request.
    pub size_bytes: u64,
    /// `None` until an owner reports it or it downloads; the playlist entry's
    /// own duration backfill on first load is what currently drives the
    /// bitrate unpause rule, so this stays reserved for now.
    pub duration_millis: Option<u64>,
}

/// AniDB relation edge types (see the UDP API's ANIME relation codes).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum RelationKind {
    /// Continues the story.
    Sequel,
    /// Precedes the story.
    Prequel,
    /// Same setting, different story.
    SameSetting,
    /// Alternative setting.
    AlternativeSetting,
    /// Alternative version of the same story.
    AlternativeVersion,
    /// Music video.
    MusicVideo,
    /// Shares characters.
    Character,
    /// Side story branching off this one.
    SideStory,
    /// The story this branches off from.
    ParentStory,
    /// Condensed retelling.
    Summary,
    /// The full story a summary condenses.
    FullStory,
    /// Anything else; carries AniDB's raw relation code.
    Other(u16),
}

impl RelationKind {
    /// Whether this edge places both series in the *same franchise*.
    ///
    /// Structural edges describe one continuous work — a sequel/prequel
    /// chain, a remake, or a spin-off/recap that branches off the main
    /// story. Non-structural edges (shared setting, shared characters,
    /// music videos, and AniDB's catch-all crossover code) link
    /// *related but separate* works: e.g. Isekai Quartet relates to
    /// Overlord, KonoSuba and Re:Zero via the crossover code, but those
    /// are four distinct franchises. Grouping on those edges collapses
    /// every crossover-linked show into one giant component.
    pub fn groups_franchise(self) -> bool {
        match self {
            RelationKind::Sequel
            | RelationKind::Prequel
            | RelationKind::AlternativeVersion
            | RelationKind::SideStory
            | RelationKind::ParentStory
            | RelationKind::Summary
            | RelationKind::FullStory => true,
            RelationKind::SameSetting
            | RelationKind::AlternativeSetting
            | RelationKind::MusicVideo
            | RelationKind::Character
            | RelationKind::Other(_) => false,
        }
    }
}

/// One related-anime edge.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct SeriesRelation {
    /// How the target relates to this series.
    pub kind: RelationKind,
    /// The related series.
    pub target: AniDbSeriesId,
}

/// Server-authoritative relations and display data for one series, fetched
/// via the AniDB ANIME command. Clients build franchise groupings from
/// these (connected components over sequel/prequel/side-story edges).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct SeriesRelations {
    /// Series title, for display.
    pub title: String,
    /// First air year, if known.
    pub year: Option<u16>,
    /// Episode count, if known.
    pub episode_count: Option<u32>,
    /// Related-anime edges.
    pub relations: BTreeSet<SeriesRelation>,
}

/// Status of an entry in The List.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum ListStatus {
    /// Up next, high priority.
    ShortList,
    /// General plan-to-watch.
    Planned,
    /// Currently being watched.
    Active,
    /// Airing now, weekly episodes.
    CurrentSeason,
    /// Waiting for release (movie, next season).
    Waiting,
    /// Paused, may resume.
    Hiatus,
    /// Done.
    Finished,
    /// Abandoned.
    Dropped,
}

/// One row of The List — the group's shared series tracker. Whole-struct
/// LWW; the fast-changing progress fields live in [`NextEpState`].
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct SeriesListEntry {
    /// Primary title.
    pub name: String,
    /// Nero's alternative title; mandatory culture.
    pub nero_name: Option<String>,
    /// Genre, free text.
    pub genre: Option<String>,
    /// Free-form notes columns.
    pub notes: Vec<String>,
    /// Who recommended it.
    pub recommender: Option<String>,
    /// Where the entry sits in the group's flow.
    pub status: ListStatus,
    /// Drop reason, hiatus progress, etc.
    pub status_note: Option<String>,
    /// Where files come from; `None` = SubsPlease/batch.
    pub source: Option<String>,
    /// Who watches this series.
    pub watchers: BTreeSet<UserId>,
    /// Linked manually after import.
    pub anidb_series_id: Option<AniDbSeriesId>,
}

/// Fast-changing progress state for a List entry, kept separate from
/// [`SeriesListEntry`] so server auto-advance never clobbers concurrent
/// note edits.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct NextEpState {
    /// Free text: "12", "S3-05", "Sisters", "movie 5?".
    pub next_ep: Option<String>,
    /// This week's episode is out (the spreadsheet's check column).
    pub available: bool,
}

/// A chat message. `Ord` orders by timestamp first, which is what the
/// GList tiebreak should reflect.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Shared-clock send time.
    pub timestamp: SharedTimestamp,
    /// Who sent it.
    pub sender: UserId,
    /// The message. An IRC-style action ("/me waves") is carried inline
    /// using the CTCP convention — `"\x01ACTION waves\x01"` — see
    /// [`encode_action`] / [`decode_action`]. This keeps actions ordinary
    /// chat messages (synced, archived, no schema change); only the
    /// display sites decode them.
    pub text: String,
}

/// The CTCP `ACTION` prefix marking an IRC-style emote ("/me waves").
const ACTION_PREFIX: &str = "\u{1}ACTION ";
/// The trailing CTCP delimiter.
const ACTION_SUFFIX: char = '\u{1}';

/// Encode an IRC-style action (`/me <action>`) into chat `text` using the
/// CTCP `ACTION` convention. The result is stored as an ordinary
/// [`ChatMessage`] text; [`decode_action`] reverses it at display time.
pub fn encode_action(action: &str) -> String {
    format!("{ACTION_PREFIX}{action}{ACTION_SUFFIX}")
}

/// If `text` is a CTCP `ACTION` (see [`encode_action`]), return the inner
/// action phrase; otherwise `None`. A missing trailing delimiter is
/// tolerated, but stripped when present.
pub fn decode_action(text: &str) -> Option<&str> {
    let body = text.strip_prefix(ACTION_PREFIX)?;
    Some(body.strip_suffix(ACTION_SUFFIX).unwrap_or(body))
}

#[cfg(test)]
mod action_tests {
    use super::*;

    #[test]
    fn round_trips() {
        assert_eq!(decode_action(&encode_action("waves")), Some("waves"));
        assert_eq!(
            decode_action(&encode_action("waves at Nero")),
            Some("waves at Nero")
        );
    }

    #[test]
    fn plain_text_is_not_an_action() {
        assert_eq!(decode_action("hello"), None);
        assert_eq!(decode_action("/me waves"), None);
        assert_eq!(decode_action(""), None);
    }

    #[test]
    fn tolerates_missing_suffix() {
        assert_eq!(decode_action("\u{1}ACTION waves"), Some("waves"));
    }
}

/// A "please look this up on AniDB" request, inserted by clients as they
/// scan local files.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct FileHashInfo {
    /// The file's ed2k root hash.
    pub hash: Ed2kHash,
    /// File size in bytes (AniDB's FILE command requires it).
    pub size: u64,
    /// For fallback metadata when AniDB doesn't know the file.
    pub filename: String,
    /// File mtime in unix millis when the requester held it locally;
    /// `None` when unknown (e.g. a playlist entry the client doesn't hold).
    /// The server anchors the re-validation backoff on the *older* of this
    /// and when it first saw the file, so files owned for years aren't
    /// re-polled on the aggressive new-file ladder after a queue reset.
    pub mtime: Option<i64>,
    /// A title-like containing-directory name (e.g. `RahXephon` for a file
    /// stored under `<root>/RahXephon/Season 1/...`), supplied by the
    /// requester when it held the file locally; `None` when unknown or when
    /// no ancestor directory looked like a title. When AniDB doesn't know the
    /// file, the server prefers this over the filename stem for the
    /// fallback series name, so a series' episodes group together instead of
    /// each becoming its own franchise.
    pub series_hint: Option<String>,
}

/// A user's playback position report.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct PlaybackPosition {
    /// Position within the file, in milliseconds.
    pub position_millis: u64,
    /// Shared-clock time the position was sampled at.
    pub timestamp: SharedTimestamp,
    /// The file this position was sampled against. Drift correction keys on
    /// this to ignore a peer's *stale* position from a previous file after a
    /// now-playing transition: `file_availability::Ready` alone is set on
    /// prefetch (see `on_resolved`), so a peer can advertise Ready for the new
    /// file while its position register still holds the previous file's
    /// position. The file tag is a clock-free identity check -- no cross-PC
    /// `SharedTimestamp` comparison, which would be unsound (see
    /// docs/sync-state.md, playback-position).
    pub file: Ed2kHash,
}
