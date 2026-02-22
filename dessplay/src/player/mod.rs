//! Player integration: trait, events, commands, and implementations.
//!
//! The [`Player`] trait abstracts over video players (mpv, vlc).
//! [`MpvPlayer`] communicates via JSON IPC over a Unix socket.
//! [`MockPlayer`] records commands for deterministic testing.
//! [`EchoFilter`] suppresses echoed events from commands we sent.

pub mod echo;
pub mod mock;
pub mod mpv;

use std::future::Future;
use std::path::Path;

use anyhow::Result;

/// Events received from the video player.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerEvent {
    /// Player was paused (by user or by us).
    Paused,
    /// Player was unpaused (by user or by us).
    Unpaused,
    /// Player seeked to a new position.
    Seeked { position_secs: f64 },
    /// Current playback position update.
    Position { position_secs: f64 },
    /// File duration became known.
    Duration { duration_secs: f64 },
    /// File finished playing.
    Eof,
    /// Player process crashed or exited unexpectedly.
    Crashed,
}

/// Commands sent to the video player.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerCommand {
    Pause,
    Unpause,
    Seek(f64),
    LoadFile(std::path::PathBuf),
    ShowOsd(String),
}

/// Abstraction over a video player (mpv, vlc).
///
/// Uses RPITIT (return-position impl trait in trait) for async methods,
/// matching the pattern used by [`dessplay_core::network::Network`].
pub trait Player: Send {
    /// Load a video file into the player.
    fn load_file(&self, path: &Path) -> impl Future<Output = Result<()>> + Send;

    /// Pause playback.
    fn pause(&self) -> impl Future<Output = Result<()>> + Send;

    /// Resume playback.
    fn unpause(&self) -> impl Future<Output = Result<()>> + Send;

    /// Seek to an absolute position in seconds.
    fn seek(&self, position_secs: f64) -> impl Future<Output = Result<()>> + Send;

    /// Get the current playback position in seconds.
    fn get_position(&self) -> impl Future<Output = Result<f64>> + Send;

    /// Get the file duration in seconds (None if not yet known).
    fn get_duration(&self) -> impl Future<Output = Result<Option<f64>>> + Send;

    /// Show an on-screen display message.
    fn show_osd(&self, text: &str, duration_ms: u64) -> impl Future<Output = Result<()>> + Send;

    /// Receive the next event from the player. Blocks until an event is available.
    fn recv_event(&self) -> impl Future<Output = Result<PlayerEvent>> + Send;

    /// Check if the player process is still alive.
    fn is_alive(&self) -> bool;

    /// Quit the player gracefully.
    fn quit(&self) -> impl Future<Output = Result<()>> + Send;
}
