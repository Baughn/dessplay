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

    /// Derive a client ActorId from a username. Never collides with
    /// [`ActorId::SERVER`].
    pub fn from_username(name: &str) -> Self {
        use digest::Digest;
        let digest = md4::Md4::digest(name.as_bytes());
        let id = u128::from_le_bytes(digest.into());
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

/// Per-user, per-series watch preference.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum SeriesWatchState {
    /// The default: this user watches the series and gates playback.
    Watching,
    /// The user skips this series and never gates playback on it.
    NotWatching,
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
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
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
    /// The message.
    pub text: String,
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
}

/// A user's playback position report.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct PlaybackPosition {
    /// Position within the file, in milliseconds.
    pub position_millis: u64,
    /// Shared-clock time the position was sampled at.
    pub timestamp: SharedTimestamp,
}
