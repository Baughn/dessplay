use serde::{Deserialize, Serialize};
use std::fmt;

/// ed2k hash = MD4 = 128 bits. Unique file identifier used across the protocol.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
pub struct FileId(pub [u8; 16]);

impl fmt::Debug for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FileId(")?;
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        write!(f, ")")
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Self-chosen username identifying a user.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
pub struct UserId(pub String);

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Connection-level peer identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PeerId(pub u64);

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Peer({})", self.0)
    }
}

/// Milliseconds since Unix epoch, adjusted via NTP-style shared clock.
/// Value 0 is reserved as a sentinel meaning "no timestamp" / "never seen"
/// and must not appear in any CRDT operation.
pub type SharedTimestamp = u64;

/// A user's self-reported readiness state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
pub enum UserState {
    Ready,
    Paused,
    NotWatching,
}

/// A user's ability to play the current file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
pub enum FileState {
    Ready,
    Missing,
    Downloading { progress: f32 },
}

// Manual PartialEq/Eq/PartialOrd/Ord impls to handle f32 (NaN-safe via total_cmp).
impl PartialEq for FileState {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for FileState {}

impl PartialOrd for FileState {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FileState {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        fn discriminant(s: &FileState) -> u8 {
            match s {
                FileState::Ready => 0,
                FileState::Missing => 1,
                FileState::Downloading { .. } => 2,
            }
        }
        let d = discriminant(self).cmp(&discriminant(other));
        if d != std::cmp::Ordering::Equal {
            return d;
        }
        match (self, other) {
            (
                FileState::Downloading { progress: a },
                FileState::Downloading { progress: b },
            ) => a.total_cmp(b),
            _ => std::cmp::Ordering::Equal,
        }
    }
}

/// Metadata retrieved from AniDB for a file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "fuzzing", derive(arbitrary::Arbitrary))]
pub struct AniDbMetadata {
    pub anime_id: u64,
    pub anime_name: String,
    pub episode_number: u32,
    pub episode_name: String,
    pub group_name: String,
}
