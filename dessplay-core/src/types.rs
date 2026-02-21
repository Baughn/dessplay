use serde::{Deserialize, Serialize};
use std::fmt;

/// ed2k hash = MD4 = 128 bits. Unique file identifier used across the protocol.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
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
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
pub struct UserId(pub String);

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Connection-level peer identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PeerId(pub u64);

/// Milliseconds since Unix epoch, adjusted via NTP-style shared clock.
pub type SharedTimestamp = u64;

/// A user's self-reported readiness state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
pub enum UserState {
    Ready,
    Paused,
    NotWatching,
}

/// A user's ability to play the current file.
#[derive(Clone, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
pub enum FileState {
    Ready,
    Missing,
    Downloading { progress: f32 },
}

// Manual Eq impl because f32 doesn't impl Eq, but our usage is safe
// (progress is always a finite value 0.0..=1.0).
impl Eq for FileState {}

/// Metadata retrieved from AniDB for a file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "fuzz", derive(arbitrary::Arbitrary))]
pub struct AniDbMetadata {
    pub anime_id: u64,
    pub anime_name: String,
    pub episode_number: u32,
    pub episode_name: String,
    pub group_name: String,
}
