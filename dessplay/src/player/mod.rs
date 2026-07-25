//! The player seam: one running media player owned by the
//! [`crate::actors::player`] actor.
//!
//! Implementations: mpv over JSON IPC ([`mpv`]) in production, the
//! in-process [`mock`] everywhere else. The trait reports **raw**
//! observations — a `PauseChanged` fires whether the user hit space or
//! we sent the command. Echo suppression (deciding which observations
//! are the user's) is the actor's job, above this seam.
//!
//! All methods take `&self`: implementations use interior mutability so
//! the actor can hold a `recv()` future in one `select!` arm while
//! commanding from another. Exactly one task should call `recv()`.

pub mod mock;
pub mod mpv;

use std::future::Future;
use std::path::Path;

/// Player-layer errors.
#[derive(Debug)]
pub enum PlayerError {
    /// The player process is gone (crashed, quit, IPC broken).
    Gone(String),
    /// Spawning or connecting to the player failed.
    Setup(String),
}

impl std::fmt::Display for PlayerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayerError::Gone(reason) => write!(f, "player gone: {reason}"),
            PlayerError::Setup(reason) => write!(f, "player setup failed: {reason}"),
        }
    }
}

impl std::error::Error for PlayerError {}

/// Something the player observed. Raw and unfiltered: echoes of our own
/// commands are included, and the actor sorts them out.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerEvent {
    /// The pause state changed (user keypress *or* our command).
    PauseChanged(bool),
    /// A seek completed; the position landed on.
    Seeked {
        /// Position after the seek, milliseconds.
        position_millis: u64,
    },
    /// Periodic position report while a file is loaded.
    Position {
        /// Current position, milliseconds.
        position_millis: u64,
    },
    /// The file's duration became known after a load.
    DurationKnown {
        /// Total duration, milliseconds.
        duration_millis: u64,
    },
    /// A loaded file finished opening and can be controlled.
    Loaded,
    /// A load **failed to open** — mpv accepted the `loadfile` command (so
    /// [`Player::load`] returned `Ok`) but the file then could not be
    /// opened (gone, unreadable, undecodable): mpv's `end-file` with
    /// `reason: "error"`. The actor maps this to the current file and
    /// reports it upstream so the session flips that file to Missing and
    /// re-resolves — the path we held may be stale (the file moved between
    /// media roots). Distinct from [`Eof`](PlayerEvent::Eof), which is a
    /// clean play-to-end.
    LoadFailed,
    /// The player's loaded file path changed (mpv's `path` property). Fires
    /// for our own `loadfile` *and* when the user loads a file directly
    /// (e.g. drag-and-drop into the mpv window); the actor decides which by
    /// comparing against the path it commanded.
    PathChanged {
        /// The path mpv now has loaded, as reported by the `path` property.
        path: String,
    },
    /// The displayed subtitle line changed (empty text = cleared).
    /// `speaker` is the ASS `Name`/actor field when present (used for
    /// optional name display and separate-pane coloring); `None` for
    /// formats without one (SRT) or events with an empty Name.
    SubtitleLine {
        /// The subtitle text, ASS override tags already stripped.
        text: String,
        /// The speaker/actor, if the cue carried one.
        speaker: Option<String>,
    },
    /// Playback reached end of file. The file stays loaded (mpv runs
    /// with `keep-open`); the server owns what happens next.
    Eof,
    /// The player process exited. Terminal: `recv` returns only errors
    /// after this.
    Exited {
        /// True for a deliberate quit, false for a crash.
        clean: bool,
    },
}

/// One running player instance.
pub trait Player: Send + Sync + 'static {
    /// Load a file, replacing whatever is playing. The player starts
    /// paused; a [`PlayerEvent::Loaded`] follows when it's ready. `title`,
    /// when given, overrides the displayed media title — needed because
    /// cached downloads are hash-named on disk (the original filename would
    /// otherwise be lost).
    fn load(
        &self,
        path: &Path,
        title: Option<&str>,
    ) -> impl Future<Output = Result<(), PlayerError>> + Send;

    /// Set the pause state.
    fn set_pause(&self, paused: bool) -> impl Future<Output = Result<(), PlayerError>> + Send;

    /// Seek to an absolute position.
    fn seek(&self, position_millis: u64) -> impl Future<Output = Result<(), PlayerError>> + Send;

    /// Set the playback speed (drift slew; 1.0 = normal).
    fn set_speed(&self, speed: f64) -> impl Future<Output = Result<(), PlayerError>> + Send;

    /// Set (or clear, with `None`) a persistent OSD overlay. `id`
    /// namespaces independent overlays (the rolling chat log vs the
    /// blocker summary); `data` is raw ASS event text (mpv
    /// `osd-overlay`). Unlike a timed `show-text`, an overlay stays
    /// until rewritten or cleared, so the two never clobber each other
    /// and nothing auto-expires under the reader.
    fn set_osd_overlay(
        &self,
        id: u64,
        data: Option<&str>,
    ) -> impl Future<Output = Result<(), PlayerError>> + Send;

    /// Ask the player to write a screenshot of the current frame to
    /// `path` (no OSD/subtitles burned in). Fire-and-forget: the write
    /// happens asynchronously in the player, so callers poll for the
    /// file rather than await a reply. The format follows the path's
    /// extension (mpv `screenshot-to-file`).
    fn screenshot_to_file(
        &self,
        path: &Path,
    ) -> impl Future<Output = Result<(), PlayerError>> + Send;

    /// Receive the next observation. Cancel-safe; one reader task.
    fn recv(&self) -> impl Future<Output = Result<PlayerEvent, PlayerError>> + Send;

    /// Quit the player deliberately.
    fn shutdown(&self) -> impl Future<Output = ()> + Send;
}

/// Spawns player instances — the actor calls it once at startup and
/// again on crash relaunch.
pub trait PlayerFactory: Send + 'static {
    /// The player type produced.
    type Player: Player;

    /// Spawn a fresh player instance.
    fn spawn(&mut self) -> impl Future<Output = Result<Self::Player, PlayerError>> + Send;

    /// True when this factory **attaches** to a player the user owns (the
    /// `--attach-mpv` dev aid) rather than spawning one we own.
    ///
    /// It changes how the actor treats a player going away. A *spawned*
    /// player dying is a real crash: relaunch, and escalate (global pause
    /// on the second death in [`CRASH_FATAL_WINDOW`], give up on the
    /// third). An *attached* player closing its socket is a transient
    /// **detach** — the user quit or restarted their own mpv — so the
    /// actor never counts it as a crash and waits (indefinitely, with
    /// capped backoff) for it to come back. See design.md, Player
    /// Integration / Attach mode.
    ///
    /// [`CRASH_FATAL_WINDOW`]: crate::actors::player::CRASH_FATAL_WINDOW
    fn is_attach(&self) -> bool {
        false
    }
}
