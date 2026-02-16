use serde::{Deserialize, Serialize};

use crate::network::clock::SharedTimestamp;
use crate::network::PeerId;

/// Monotonically increasing per-user sequence number.
pub type SequenceNumber = u64;

/// Stable identifier for a playlist item, assigned on creation.
///
/// Combines the creator's peer ID with a per-user sequence number,
/// ensuring globally unique IDs without coordination.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ItemId {
    pub user: PeerId,
    pub seq: SequenceNumber,
}

impl std::fmt::Display for ItemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.user, self.seq)
    }
}

/// Per-user readiness state. Each user controls their own state only.
/// Synced as a per-user LWW register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserState {
    Ready,
    Paused,
    NotWatching,
}

/// Per-user file availability state. Describes ability to play the current file.
/// Synced as a per-user LWW register.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum FileState {
    Ready,
    Missing,
    Downloading {
        /// Download progress [0.0, 1.0].
        progress: f32,
        /// Whether download speed exceeds the file's bitrate.
        speed_sufficient: bool,
    },
}

impl Eq for FileState {}


impl FileState {
    /// Whether this file state permits playback to proceed.
    pub fn permits_playback(&self) -> bool {
        match self {
            FileState::Ready => true,
            FileState::Missing => false,
            FileState::Downloading {
                progress,
                speed_sufficient,
            } => *speed_sufficient && *progress >= 0.2,
        }
    }
}

/// Derived ready state for UI display. Combines UserState and FileState.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyState {
    /// Ready & Ready
    Ready,
    /// Paused & Any
    Paused,
    /// NotWatching & Any
    NotWatching,
    /// Ready & Downloading (complete enough to play)
    DownloadingReady,
    /// Any & Downloading (not ready to play)
    DownloadingNotReady,
}

impl ReadyState {
    /// Derive ready state from user state and file state.
    pub fn derive(user_state: UserState, file_state: FileState) -> Self {
        match user_state {
            UserState::Paused => ReadyState::Paused,
            UserState::NotWatching => ReadyState::NotWatching,
            UserState::Ready => match file_state {
                FileState::Ready => ReadyState::Ready,
                FileState::Missing => ReadyState::Paused, // can't play = effectively paused
                FileState::Downloading { .. } if file_state.permits_playback() => {
                    ReadyState::DownloadingReady
                }
                FileState::Downloading { .. } => ReadyState::DownloadingNotReady,
            },
        }
    }
}

/// Playback position with a timestamp for LWW merge.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PositionRegister {
    /// Playback position in seconds.
    pub position: f64,
    /// Shared clock timestamp when this position was recorded.
    pub timestamp: SharedTimestamp,
}

/// Snapshot of the player's state. LWW register.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerStateSnapshot {
    /// Currently loaded file, if any.
    pub file_id: Option<ItemId>,
    /// Playback position.
    pub position: PositionRegister,
}

/// An action on the playlist. Entries in the per-user playlist append log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PlaylistAction {
    Add {
        id: ItemId,
        filename: String,
        /// Insert after this item. None = insert at beginning.
        after: Option<ItemId>,
    },
    Remove {
        id: ItemId,
    },
    Move {
        id: ItemId,
        /// Move to after this item. None = move to beginning.
        after: Option<ItemId>,
    },
}

/// A chat message. Entries in the per-user chat append log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub sender: PeerId,
    pub text: String,
    pub timestamp: SharedTimestamp,
}

/// A resolved playlist item (result of replaying playlist actions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaylistItem {
    pub id: ItemId,
    pub filename: String,
}

/// Discriminator for append log types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LogType {
    Chat,
    Playlist,
}
