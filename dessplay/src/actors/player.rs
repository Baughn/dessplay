//! The player actor: owns the running player (mpv in production, a mock
//! in tests), translating between raw player observations and the
//! session's view of them.
//!
//! Responsibilities (design.md, Playback Rules / Player Integration):
//!
//! - **Echo suppression.** The player reports every pause flip and seek,
//!   including the ones we commanded. Commands are remembered (a queue
//!   of expected pause states, a counter of expected seeks) and matching
//!   observations are swallowed; only genuine user actions surface as
//!   [`PlayerOutput`]s.
//! - **Observe and correct, never enforce.** A user unpause is reported
//!   upstream and *not* locally reverted: the main loop re-derives
//!   gating and sends [`PlayerCommand::SetPlaying`] back down, which
//!   re-pauses if someone blocks. (That's the design's "your player is
//!   immediately re-paused".)
//! - **Drift correction** against the seek authority's extrapolated
//!   position. The banding, hysteresis, and slew tapering live in
//!   [`DriftController`] (see [`super::drift`] for why its shape is
//!   dictated by what is audible); this actor computes the delta and
//!   applies the returned action.
//! - **Seek debounce.** User scrubbing is only reported
//!   [`SEEK_DEBOUNCE`] after the last seek; drift correction is
//!   suspended while a debounce is pending (the scrubber is about to
//!   become the authority).
//! - **Position cadence.** Emits [`PlayerOutput::PositionTick`] every
//!   100ms while playing, every 1s while paused.
//! - **Evidence-based file attribution.** `loadfile` is asynchronous:
//!   after a `Load` command the player stays on (and keeps reporting
//!   positions, seeks, even the EOF of) the *previous* file until the
//!   new one actually opens — a long window on a slow machine.
//!   File-attributed observations are accepted only while the *observed*
//!   `path` property, which arrives in order with every other event,
//!   confirms the player holds the commanded file; in the gap they
//!   belong to the old file and are dropped. Without this, a trailing
//!   old-file position was broadcast under the new file's identity and
//!   the group latched onto it (2026-07-27).
//! - **Crash supervision.** A dead player is relaunched and restored
//!   (same file, last position, desired pause state). The relaunch runs
//!   in a background task — mpv can take its full 30s socket wait to
//!   come up, and awaiting that inline would park the actor (and, via
//!   the bounded command channel, the session's main loop) for the
//!   whole wait. A second death
//!   within [`CRASH_FATAL_WINDOW`] additionally emits
//!   [`PlayerOutput::FatalCrash`], which the main loop turns into a
//!   global pause and a chat notice — the relaunch then comes up paused,
//!   which is exactly the safe state if the file itself is crashing the
//!   player.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use dessplay_core::types::{Ed2kHash, SharedTimestamp};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use super::drift::{DriftAction, DriftController};
use super::network::Clock;
use crate::player::{Player, PlayerError, PlayerEvent, PlayerFactory};

pub use super::drift::{
    DRIFT_ENGAGE_MILLIS, DRIFT_HARD_SEEK_MILLIS, DRIFT_RELEASE_MILLIS, SLEW_RATE,
};
/// A user seek is reported this long after the *last* seek — scrubbing
/// coalesces into one authority change.
pub const SEEK_DEBOUNCE: Duration = Duration::from_millis(1500);
/// Position broadcast cadence while playing.
pub const POSITION_CADENCE_PLAYING: Duration = Duration::from_millis(100);
/// Position broadcast cadence while paused.
pub const POSITION_CADENCE_PAUSED: Duration = Duration::from_secs(1);
/// A second player death within this window is a fatal crash loop.
pub const CRASH_FATAL_WINDOW: Duration = Duration::from_secs(30);
/// This many deaths in a row, each within [`CRASH_FATAL_WINDOW`] of the
/// last, and the actor gives up relaunching until a new file is loaded —
/// otherwise a file that reliably kills the player loops forever.
pub const CRASH_GIVE_UP_COUNT: u32 = 3;

/// mpv overlay slot for the rolling chat-message log.
const OSD_CHAT_OVERLAY_ID: u64 = 1;
/// mpv overlay slot for the "Waiting for …" blocker summary.
const OSD_BLOCKER_OVERLAY_ID: u64 = 2;
/// Minimum time each chat message stays on the OSD. Messages expire
/// individually, so a burst never erases an unread line early (#16).
pub const OSD_CHAT_RETENTION: Duration = Duration::from_secs(8);
/// Upper bound on simultaneously shown chat messages — a guard against
/// pathological bursts, not a display budget (older lines have had the
/// least-recent chance to be read).
const OSD_CHAT_MAX: usize = 8;
/// Attach mode: first delay before re-probing the user's mpv socket after
/// it goes away (it usually comes back quickly).
pub const REATTACH_BACKOFF_INITIAL: Duration = Duration::from_millis(500);
/// Attach mode: the re-attach backoff never grows past this.
pub const REATTACH_BACKOFF_MAX: Duration = Duration::from_secs(10);
/// Attach mode: how long a single re-attach probe may run before it is
/// abandoned and rescheduled. An attach-mode spawn does `wait_for_socket`,
/// which otherwise loops against its own ~10s deadline; awaiting that inline
/// in the run loop's `select!` would park the whole actor (Shutdown, Load,
/// SyncTo, the cadence tick) for the full wait. A short bound keeps the loop
/// responsive — a live socket attaches well within it; a dead one fails fast
/// and backs off.
pub const REATTACH_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Commands from the main loop.
#[derive(Debug)]
pub enum PlayerCommand {
    /// Load a file (it starts paused; desired state is applied on load).
    Load {
        /// The file's identity, echoed back in [`PlayerOutput::Eof`].
        file: Ed2kHash,
        /// Local path to play.
        path: PathBuf,
        /// Display title override. Cache files are hash-named on disk, so
        /// this carries the real filename for mpv to show.
        title: Option<String>,
    },
    /// The derived group playback state: should video be running?
    /// Sent on every re-derivation; the actor dedups against what the
    /// player is already doing.
    SetPlaying(bool),
    /// The seek authority's latest position sample. The actor
    /// extrapolates it to now and applies the drift bands. Never sent
    /// when we *are* the authority.
    SyncTo {
        /// Sampled position, milliseconds.
        position_millis: u64,
        /// Shared-clock time of the sample.
        timestamp: SharedTimestamp,
        /// Whether video is running (extrapolation only applies then).
        playing: bool,
    },
    /// Release any in-progress drift slew — reset playback rate to 1.0.
    /// Drift correction is reactive (it only touches speed inside a
    /// `SyncTo`); when the position reference drops (the followed peer
    /// departs and we become the leader, or we take seek authority) no
    /// `SyncTo` is emitted, so nothing else restores the rate.
    ReleaseSlew,
    /// Updated shared-clock offset (server minus local), from time sync.
    ClockOffset(i64),
    /// Append a chat message to the rolling OSD log (it stays at least
    /// [`OSD_CHAT_RETENTION`], alongside the other recent messages).
    ShowOsd(String),
    /// Set (or clear) the persistent "Waiting for …" blocker summary.
    /// Plain text; the actor owns the ASS formatting and re-applies the
    /// overlay across player relaunches.
    SetBlockerOverlay(Option<String>),
    /// Write a screenshot of the current frame to this path
    /// (best-effort: dropped silently when no player is running — the
    /// requester polls for the file and proceeds without it).
    Screenshot(PathBuf),
    /// Quit the player and exit the actor.
    Shutdown,
}

/// What the actor reports upstream. Echoes of our own commands are
/// already filtered out.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerOutput {
    /// The user paused in the player.
    UserPaused,
    /// The user unpaused in the player.
    UserUnpaused,
    /// The user seeked (debounced; scrubbing already coalesced).
    UserSeeked {
        /// Position when the debounced scrub began.
        from_millis: u64,
        /// Final position after the debounce settled.
        to_millis: u64,
    },
    /// Periodic position report (100ms playing / 1s paused).
    PositionTick {
        /// The file this position measures (the player's loaded file).
        /// A position is meaningless without it: after an EOF-advance the
        /// player can still be on the previous file, and a trailing tick
        /// must not be attributed to the new now-playing file.
        file: Ed2kHash,
        /// Current position, milliseconds.
        position_millis: u64,
    },
    /// The loaded file's duration became known.
    DurationKnown {
        /// Which file.
        file: Ed2kHash,
        /// Duration, milliseconds.
        duration_millis: u64,
    },
    /// The player's displayed subtitle line changed.
    SubtitleLine {
        /// The subtitle text (empty = the previous cue cleared).
        text: String,
        /// The ASS `Name`/actor field, if the cue carried one (never
        /// displayed — used only to color the line).
        speaker: Option<crate::player::SpeakerName>,
        /// In-video position when the cue appeared (milliseconds); the
        /// displayed timestamp. `0` before the first position sample.
        position_millis: u64,
    },
    /// Playback reached end of file.
    Eof {
        /// Which file ended.
        file: Ed2kHash,
    },
    /// The player could not load the file (e.g. the path is gone or
    /// unreadable). The session flips it to Missing and re-resolves.
    LoadFailed {
        /// The file that failed to load.
        file: Ed2kHash,
    },
    /// The user loaded a file directly into the player (drag-and-drop) — a
    /// path we never commanded. The session may adopt it to clear a Missing
    /// now-playing file when the name matches. Echoes of our own loads
    /// (including the placeholder) are already filtered out.
    PathObserved {
        /// The path the user loaded.
        path: PathBuf,
    },
    /// The player died twice within [`CRASH_FATAL_WINDOW`]. The main
    /// loop should pause globally and say so in chat.
    FatalCrash,
    /// The player crashed [`CRASH_GIVE_UP_COUNT`] times in a row; the
    /// actor stopped relaunching until a new file is loaded. The main
    /// loop should pause globally and say so in chat.
    GaveUp,
}

/// Position estimate: last observation plus extrapolation while playing.
struct Estimate {
    millis: u64,
    at: Instant,
}

struct Actor<F: PlayerFactory> {
    /// `None` exactly while a spawn-mode launch is in flight (the factory
    /// travels with the background task and comes back with the result;
    /// see [`Actor::begin_spawn`]). Always `Some` in attach mode.
    factory: Option<F>,
    outputs: mpsc::Sender<PlayerOutput>,
    clock: Clock,
    offset_millis: i64,
    player: Option<F::Player>,
    /// Now-loaded file, path, and display title (for EOF attribution and
    /// relaunch — the title must be re-applied so a relaunched player still
    /// shows the real filename for hash-named cache files).
    current: Option<(Ed2kHash, PathBuf, Option<String>)>,
    /// The path the player itself most recently *reported* loaded (the
    /// observed `path` property) — evidence, where `current` is belief
    /// (what we last commanded). The two diverge from a `Load` command
    /// until the player's path echo arrives, and while the user has
    /// loaded their own file; see [`Actor::player_on_current_file`].
    observed_path: Option<PathBuf>,
    /// The group's desired playback state.
    desired_playing: bool,
    /// Pause state we believe the player is in (last command or
    /// observation); `None` until something is known.
    believed_pause: Option<bool>,
    /// Current slew speed (1.0 = not slewing).
    speed: f64,
    /// Drift-correction controller (banding, hysteresis, slew taper).
    drift: DriftController,
    /// Pause flips we commanded and haven't seen echoed yet.
    pending_pause_echoes: VecDeque<bool>,
    /// Seeks we commanded and haven't seen echoed yet.
    pending_seek_echoes: usize,
    /// A user seek waiting out the debounce window.
    /// Debounced user scrub: initial position, latest destination, last move.
    pending_user_seek: Option<(u64, u64, Instant)>,
    /// Restore-after-relaunch position.
    restore_millis: Option<u64>,
    estimate: Option<Estimate>,
    last_position_emit: Option<Instant>,
    eof_reported: bool,
    last_death: Option<Instant>,
    /// Deaths in a row, each within [`CRASH_FATAL_WINDOW`] of the last.
    /// Reset when a different file is loaded. Spawn mode only.
    consecutive_crashes: u32,
    /// True when the player is an *attached* user-owned mpv rather than
    /// one we spawned (see [`PlayerFactory::is_attach`]). A death is then a
    /// transient detach to wait out, never a crash to escalate.
    attach_mode: bool,
    /// Attach mode: when `Some`, the player is gone and the actor is
    /// waiting to re-attach at this instant. The run loop drives the
    /// retry; `None` whenever a player is connected.
    reattach_at: Option<Instant>,
    /// Attach mode: current delay between re-attach attempts (capped
    /// backoff), reset to [`REATTACH_BACKOFF_INITIAL`] on a fresh detach.
    reattach_backoff: Duration,
    /// Spawn mode: a launch running in its own background task —
    /// `factory.spawn()` may legitimately take up to mpv's 30s socket
    /// wait (a slow startup is not a crash, 7e7ffc9), and awaiting it
    /// inline in the run loop's `select!` would park the actor (no
    /// Shutdown, no SetPlaying) for the whole wait, backing the
    /// session's bounded player channel up into its main loop. The run
    /// loop polls this for the result; the factory rides along so it is
    /// available for the next relaunch.
    pending_spawn: Option<oneshot::Receiver<Spawned<F>>>,
    /// Whether a pending spawn's failure exits the actor (startup and
    /// crash relaunch, matching the old inline paths) or leaves it idle
    /// awaiting another Load (the give-up recovery path).
    spawn_failure_exits: bool,
    /// The rolling chat OSD: `(rendered line, expires_at)`, oldest
    /// first. Retention is constant, so the front always expires first.
    osd_chat: VecDeque<(String, Instant)>,
    /// The current blocker summary (plain text), kept so it survives a
    /// player relaunch.
    blocker_overlay: Option<String>,
}

/// Run the player actor until `commands` closes or [`PlayerCommand::Shutdown`].
pub async fn run<F: PlayerFactory>(
    factory: F,
    clock: Clock,
    mut commands: mpsc::Receiver<PlayerCommand>,
    outputs: mpsc::Sender<PlayerOutput>,
) {
    let attach_mode = factory.is_attach();
    let mut actor = Actor {
        factory: Some(factory),
        outputs,
        clock,
        offset_millis: 0,
        player: None,
        current: None,
        observed_path: None,
        desired_playing: false,
        believed_pause: None,
        speed: 1.0,
        drift: DriftController::new(),
        pending_pause_echoes: VecDeque::new(),
        pending_seek_echoes: 0,
        pending_user_seek: None,
        restore_millis: None,
        estimate: None,
        last_position_emit: None,
        eof_reported: false,
        last_death: None,
        consecutive_crashes: 0,
        attach_mode,
        reattach_at: None,
        reattach_backoff: REATTACH_BACKOFF_INITIAL,
        pending_spawn: None,
        spawn_failure_exits: false,
        osd_chat: VecDeque::new(),
        blocker_overlay: None,
    };

    // Never await a spawn inline: mpv can take its full 30s socket wait
    // to come up (a slow startup is not a crash), and commands (Load,
    // Shutdown) must flow meanwhile. Attach mode goes through the same
    // bounded probe as any re-attach (the user's mpv may not be up yet;
    // design.md: attach mode waits for it to appear); spawn mode
    // launches in a background task the loop polls.
    if attach_mode {
        actor.begin_reattach();
    } else {
        actor.begin_spawn(true);
    }

    let mut cadence = tokio::time::interval(POSITION_CADENCE_PLAYING);
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let debounce_at = actor.pending_user_seek.map(|(_, _, at)| at + SEEK_DEBOUNCE);
        let reattach_at = actor.reattach_at;
        // Constant retention means the oldest chat line expires first.
        let osd_expiry_at = actor.osd_chat.front().map(|(_, at)| *at);
        tokio::select! {
            cmd = commands.recv() => {
                let Some(cmd) = cmd else { break };
                let quit = matches!(cmd, PlayerCommand::Shutdown);
                actor.handle_command(cmd).await;
                if quit {
                    break;
                }
            }
            event = recv_from(&actor.player), if actor.player.is_some() => {
                if !actor.handle_player_event(event).await {
                    break;
                }
            }
            // Spawn mode: a background launch finished (see begin_spawn).
            spawned = recv_spawned(&mut actor.pending_spawn),
                if actor.pending_spawn.is_some() =>
            {
                if !actor.finish_spawn(spawned).await {
                    break;
                }
            }
            _ = cadence.tick() => {
                actor.maybe_emit_position().await;
            }
            _ = tokio::time::sleep_until(debounce_at.unwrap_or_else(Instant::now)),
                if debounce_at.is_some() =>
            {
                actor.flush_user_seek().await;
            }
            // Attach mode: re-probe the user's mpv socket. While waiting,
            // the player is None so the event arm is idle and commands
            // (Shutdown, Load) still flow.
            _ = tokio::time::sleep_until(reattach_at.unwrap_or_else(Instant::now)),
                if reattach_at.is_some() =>
            {
                actor.try_reattach().await;
            }
            _ = tokio::time::sleep_until(osd_expiry_at.unwrap_or_else(Instant::now)),
                if osd_expiry_at.is_some() =>
            {
                actor.expire_osd_chat().await;
            }
        }
    }
    if let Some(player) = &actor.player {
        player.shutdown().await;
    }
    tracing::debug!("player actor exiting");
}

async fn recv_from<P: Player>(player: &Option<P>) -> Result<PlayerEvent, PlayerError> {
    match player {
        Some(p) => p.recv().await,
        // Unreachable: the select arm is gated on `is_some`.
        None => std::future::pending().await,
    }
}

/// A background launch's delivery: the factory riding back for the
/// next relaunch, plus the launch result.
type Spawned<F> = (F, Result<<F as PlayerFactory>::Player, PlayerError>);

/// Await a pending background spawn's delivery. `None` when the spawn
/// task died without delivering (a panic in the factory).
async fn recv_spawned<F: PlayerFactory>(
    pending: &mut Option<oneshot::Receiver<Spawned<F>>>,
) -> Option<Spawned<F>> {
    match pending {
        Some(rx) => rx.await.ok(),
        // Unreachable: the select arm is gated on `is_some`.
        None => std::future::pending().await,
    }
}

impl<F: PlayerFactory> Actor<F> {
    fn shared_now(&self) -> u64 {
        (self.clock)().saturating_add_signed(self.offset_millis)
    }

    /// Best estimate of the player's current position.
    fn estimate_now(&self) -> Option<u64> {
        let est = self.estimate.as_ref()?;
        if self.believed_pause == Some(false) {
            let elapsed = est.at.elapsed().as_millis() as f64 * self.speed;
            Some(est.millis + elapsed as u64)
        } else {
            Some(est.millis)
        }
    }

    fn note_position(&mut self, millis: u64) {
        self.estimate = Some(Estimate {
            millis,
            at: Instant::now(),
        });
    }

    /// Freeze the position estimate at its current extrapolated value.
    ///
    /// `estimate_now` reads `believed_pause` and `speed`, so mutating
    /// either retroactively re-interprets the interval since the last
    /// anchor — a pause→play flip would otherwise count the whole paused
    /// gap as playback. Call this **before** changing pause/speed so the
    /// change only affects the future, never the elapsed interval. (The
    /// invariant lives in one place precisely because every site that
    /// forgot it re-introduces that phantom-playback jump.)
    fn reanchor_estimate(&mut self) {
        if let Some(now) = self.estimate_now() {
            self.note_position(now);
        }
    }

    /// True iff the player has confirmed — via the observed `path`
    /// property, which arrives in order with every other event — that
    /// the file it has loaded is the one we last commanded. `loadfile`
    /// is asynchronous, so between a `Load` command and the player's
    /// path echo the player is still on the *previous* file: every
    /// file-attributed observation in that gap (position, seek, EOF,
    /// duration) describes the old file, while `current` — which tags
    /// them — already names the new one. Attributing across the gap
    /// broadcast a late old-episode position under the new file's
    /// identity, and forward-only leader election latched the group
    /// onto it (2026-07-27; long loads made the gap wide). Also false
    /// while the user's own file (drag-and-drop) is up.
    fn player_on_current_file(&self) -> bool {
        match (&self.observed_path, &self.current) {
            (Some(observed), Some((_, commanded, _))) => observed == commanded,
            _ => false,
        }
    }

    async fn handle_command(&mut self, cmd: PlayerCommand) {
        match cmd {
            PlayerCommand::Load { file, path, title } => {
                tracing::info!(path = %path.display(), "loading file");
                let different = self.current.as_ref().map(|(f, ..)| *f) != Some(file);
                self.current = Some((file, path.clone(), title.clone()));
                self.eof_reported = false;
                self.restore_millis = None;
                self.pending_user_seek = None;
                // A loadfile replaces the file/position, so any pause/seek
                // echoes still awaited from the previous file'''s commands are
                // moot -- and a leftover seek echo would silently swallow the
                // user'''s next real seek on the new file. Clear them like
                // handle_player_death does.
                self.pending_pause_echoes.clear();
                self.pending_seek_echoes = 0;
                self.estimate = None;
                // The load contract says the file opens paused.
                self.believed_pause = Some(true);
                self.drift.reset();
                self.set_speed(1.0).await;
                if different {
                    // A new file is a clean slate for the crash-loop counter.
                    self.consecutive_crashes = 0;
                    self.last_death = None;
                    if self.player.is_none()
                        && self.reattach_at.is_none()
                        && self.pending_spawn.is_none()
                    {
                        // We gave up relaunching after a crash loop; a new
                        // file is the recovery trigger — bring a player back.
                        // (Skipped while a re-attach or a launch is already
                        // pending: it will pick up this new `current`.)
                        tracing::info!("relaunching player for the new file");
                        self.begin_spawn(false);
                    }
                }
                if let Some(player) = &self.player
                    && let Err(e) = player.load(&path, title.as_deref()).await
                {
                    // A failed load is not silent: tell the session so it
                    // flips the file to Missing and re-resolves (the file
                    // may have been deleted under us).
                    tracing::warn!(path = %path.display(), "load failed: {e}");
                    let _ = self.outputs.send(PlayerOutput::LoadFailed { file }).await;
                }
            }
            PlayerCommand::SetPlaying(playing) => {
                self.desired_playing = playing;
                self.apply_desired_pause().await;
            }
            PlayerCommand::SyncTo {
                position_millis,
                timestamp,
                playing,
            } => {
                self.drift_correct(position_millis, timestamp, playing)
                    .await;
            }
            PlayerCommand::ReleaseSlew => {
                // Idempotent: set_speed early-returns when already 1.0.
                self.drift.reset();
                self.set_speed(1.0).await;
            }
            PlayerCommand::ClockOffset(offset_millis) => {
                self.offset_millis = offset_millis;
            }
            PlayerCommand::ShowOsd(text) => {
                self.osd_chat
                    .push_back((text, Instant::now() + OSD_CHAT_RETENTION));
                while self.osd_chat.len() > OSD_CHAT_MAX {
                    self.osd_chat.pop_front();
                }
                self.render_chat_overlay().await;
            }
            PlayerCommand::SetBlockerOverlay(text) => {
                self.blocker_overlay = text;
                self.render_blocker_overlay().await;
            }
            PlayerCommand::Screenshot(path) => {
                if let Some(player) = &self.player
                    && let Err(e) = player.screenshot_to_file(&path).await
                {
                    tracing::debug!(path = %path.display(), "screenshot failed: {e}");
                }
            }
            PlayerCommand::Shutdown => {}
        }
    }

    /// Escape a plain text line for use inside an ASS overlay event:
    /// override-block braces are swapped for parens (ASS has no reliable
    /// in-band escape for them) and newlines become spaces (one visual
    /// line per message; `\N` is reserved for joining messages).
    fn ass_escape(text: &str) -> String {
        text.replace('{', "(")
            .replace('}', ")")
            .replace(['\n', '\r'], " ")
            .replace('\\', "\u{ff3c}") // full-width \: a bare one starts an ASS tag
    }

    /// Push (or clear) one overlay slot on the running player, if any.
    async fn apply_overlay(&mut self, id: u64, data: Option<String>) {
        if let Some(player) = &self.player
            && let Err(e) = player.set_osd_overlay(id, data.as_deref()).await
        {
            tracing::debug!(id, "osd overlay failed: {e}");
        }
    }

    /// Render the rolling chat log into its overlay slot: top-left,
    /// oldest first, one line per message.
    async fn render_chat_overlay(&mut self) {
        let data = (!self.osd_chat.is_empty()).then(|| {
            let lines: Vec<String> = self
                .osd_chat
                .iter()
                .map(|(text, _)| Self::ass_escape(text))
                .collect();
            format!("{{\\an7\\fs26}}{}", lines.join("\\N"))
        });
        self.apply_overlay(OSD_CHAT_OVERLAY_ID, data).await;
    }

    /// Render the blocker summary into its overlay slot: top-right, one
    /// line, present exactly while someone blocks.
    async fn render_blocker_overlay(&mut self) {
        let data = self
            .blocker_overlay
            .as_deref()
            .map(|text| format!("{{\\an9\\fs26}}{}", Self::ass_escape(text)));
        self.apply_overlay(OSD_BLOCKER_OVERLAY_ID, data).await;
    }

    /// Drop expired chat lines and re-render.
    async fn expire_osd_chat(&mut self) {
        let now = Instant::now();
        while self.osd_chat.front().is_some_and(|(_, at)| *at <= now) {
            self.osd_chat.pop_front();
        }
        self.render_chat_overlay().await;
    }

    /// Re-push overlays onto a freshly (re)launched player — a new mpv
    /// process starts with clean overlay slots, so empty ones need no
    /// clearing command.
    async fn reapply_overlays(&mut self) {
        if !self.osd_chat.is_empty() {
            self.render_chat_overlay().await;
        }
        if self.blocker_overlay.is_some() {
            self.render_blocker_overlay().await;
        }
    }

    /// Command the player toward the desired pause state, if it isn't
    /// already there (or already being told to go there).
    async fn apply_desired_pause(&mut self) {
        let target = !self.desired_playing;
        if self.current.is_none() || self.believed_pause == Some(target) {
            return;
        }
        if let Some(player) = &self.player {
            tracing::debug!(pause = target, "commanding pause state");
            if player.set_pause(target).await.is_ok() {
                // Settle the estimate at the flip: extrapolation stops
                // (or starts) from the position at this instant.
                self.reanchor_estimate();
                self.believed_pause = Some(target);
                self.pending_pause_echoes.push_back(target);
            }
        }
    }

    async fn set_speed(&mut self, speed: f64) {
        if (self.speed - speed).abs() < f64::EPSILON {
            return;
        }
        if let Some(player) = &self.player
            && player.set_speed(speed).await.is_ok()
        {
            // Re-anchor so past extrapolation used the old speed.
            self.reanchor_estimate();
            self.speed = speed;
        }
    }

    async fn seek_programmatic(&mut self, target_millis: u64) {
        if let Some(player) = &self.player
            && player.seek(target_millis).await.is_ok()
        {
            self.pending_seek_echoes += 1;
            self.note_position(target_millis);
        }
    }

    /// Apply the drift bands against the authority's sample.
    async fn drift_correct(&mut self, sample: u64, timestamp: SharedTimestamp, playing: bool) {
        // The same evidence gate as every other file-attributed
        // operation: the authority's sample speaks about `current`, and
        // until the player confirms it holds that file (mid-load, or the
        // user's own drag-in is up) a correction would slew or hard-seek
        // the wrong video. Reset the controller too, so a run
        // accumulated before the gate closed cannot fire off the stale
        // estimate the moment the gate re-opens (belt to the foreign
        // `PathChanged` reset's suspenders).
        if !self.player_on_current_file() {
            self.drift.reset();
            return;
        }
        if self.pending_user_seek.is_some() {
            // The user is scrubbing; they're about to become authority.
            return;
        }
        let Some(current) = self.estimate_now() else {
            return;
        };
        let target = if playing {
            sample.saturating_add(self.shared_now().saturating_sub(timestamp.0))
        } else {
            sample
        };
        let delta = target as i64 - current as i64;
        match self.drift.observe(delta) {
            DriftAction::None => {}
            DriftAction::SetSpeed(slew) => {
                tracing::debug!(delta, slew, "drift: slewing");
                self.set_speed(slew).await;
            }
            DriftAction::HardSeek => {
                tracing::info!(delta, target, "drift: hard seek");
                self.set_speed(1.0).await;
                self.seek_programmatic(target).await;
            }
        }
    }

    /// Returns false when the actor should exit.
    async fn handle_player_event(&mut self, event: Result<PlayerEvent, PlayerError>) -> bool {
        let event = match event {
            Ok(event) => event,
            Err(e) => {
                tracing::warn!("player connection lost: {e}");
                return self.handle_player_death(false).await;
            }
        };
        tracing::trace!(?event, "player event");
        match event {
            PlayerEvent::PauseChanged(paused) => {
                self.handle_pause_observation(paused).await;
            }
            PlayerEvent::Seeked { position_millis } => {
                // An outstanding programmatic echo is consumed regardless
                // of attribution — it is *our own* seek coming back, and
                // an unconsumed echo would swallow the user's next
                // genuine seek as stale (the counter is otherwise only
                // cleared by a Load or a player death).
                let echo = self.pending_seek_echoes > 0;
                if echo {
                    self.pending_seek_echoes -= 1;
                }
                // A seek observed before the player confirms our latest
                // `Load` landed on the *previous* file (a trailing echo,
                // or the reply to a position query issued on it) — it
                // must neither anchor the estimate nor debounce into a
                // `UserSeeked`, which would seize seek authority for the
                // new file at the old file's position.
                if !self.player_on_current_file() {
                    tracing::debug!(position_millis, "seek for a file no longer loaded; dropped");
                } else {
                    self.eof_reported = false;
                    let from_millis = self
                        .pending_user_seek
                        .map(|(from, _, _)| from)
                        .or_else(|| self.estimate_now())
                        .unwrap_or(position_millis);
                    self.note_position(position_millis);
                    if !echo {
                        tracing::debug!(position_millis, "user seek (debouncing)");
                        self.pending_user_seek =
                            Some((from_millis, position_millis, Instant::now()));
                    }
                }
            }
            PlayerEvent::Position { position_millis } => {
                // Only positions the player has confirmed are for
                // `current` may anchor the estimate; a trailing old-file
                // sample after a `Load` must not poison it.
                if self.player_on_current_file() {
                    self.note_position(position_millis);
                }
            }
            PlayerEvent::DurationKnown { duration_millis } => {
                if self.player_on_current_file()
                    && let Some((file, _, _)) = &self.current
                {
                    let _ = self
                        .outputs
                        .send(PlayerOutput::DurationKnown {
                            file: *file,
                            duration_millis,
                        })
                        .await;
                }
            }
            PlayerEvent::Loaded => {
                self.believed_pause = Some(true);
                // A normal load starts at zero. Seed the estimate so an
                // immediate first user seek still has an honest `from` even
                // before mpv's first time-pos observation arrives.
                if self.estimate.is_none() {
                    self.note_position(0);
                }
                if let Some(millis) = self.restore_millis.take() {
                    self.seek_programmatic(millis).await;
                }
                self.apply_desired_pause().await;
            }
            PlayerEvent::PathChanged { path } => {
                let observed = PathBuf::from(path);
                // Evidence first: this is the confirmation
                // `player_on_current_file` waits for after a `Load` —
                // from here on, observations are attributed to `current`.
                self.observed_path = Some(observed.clone());
                // Swallow the echo of our own load (a real file or the
                // placeholder); any other path is one the user loaded
                // directly (drag-and-drop). The session decides whether to
                // adopt it — `self.current.path` is exactly what we last
                // commanded, so a string-equal path is our own.
                let ours = self
                    .current
                    .as_ref()
                    .is_some_and(|(_, commanded, _)| *commanded == observed);
                if !ours {
                    tracing::info!(
                        path = %observed.display(),
                        "user loaded a file directly into the player"
                    );
                    // The attribution gate just closed: forget any drift
                    // run in progress, so a stale near-complete run can't
                    // fire a correction the moment the gate re-opens.
                    // (Load and player death already reset on their gate
                    // closures.)
                    self.drift.reset();
                    let _ = self
                        .outputs
                        .send(PlayerOutput::PathObserved { path: observed })
                        .await;
                }
            }
            PlayerEvent::SubtitleLine { text, speaker } => {
                // Capture the in-video position here, where the estimate
                // is freshest; `0` (-> 00:00) is honest before the first
                // position sample.
                let position_millis = self.estimate_now().unwrap_or(0);
                let _ = self
                    .outputs
                    .send(PlayerOutput::SubtitleLine {
                        text,
                        speaker,
                        position_millis,
                    })
                    .await;
            }
            PlayerEvent::Eof => {
                // The old file reaching its end just as the next episode's
                // `Load` is commanded must not report EOF *of the new file*
                // (the server would instantly advance past it and mark it
                // watched) — hence the same evidence gate as positions.
                if !self.eof_reported
                    && self.player_on_current_file()
                    && let Some((file, _, _)) = &self.current
                {
                    self.eof_reported = true;
                    tracing::info!("end of file reached");
                    let _ = self.outputs.send(PlayerOutput::Eof { file: *file }).await;
                }
            }
            PlayerEvent::LoadFailed => {
                // mpv accepted the loadfile (so `load()` returned Ok) but the
                // file could not be opened — the path we held is likely stale
                // (the file moved between media roots). Report it so the
                // session forgets the local copy, flips the file to Missing,
                // and re-resolves; without this the group unpaused on a file
                // mpv never loaded, showing only the forced media title.
                //
                // Deliberately NOT gated on `player_on_current_file`: a file
                // that fails to *open* may never produce a path observation
                // at all, and dropping the report would leave the session
                // believing a load that never happened — the very bug this
                // event exists to fix. A stale old-load error misattributed
                // to `current` merely makes it re-resolve: wrong but
                // self-healing, the safe failure direction.
                if let Some((file, _, _)) = &self.current {
                    tracing::warn!("player failed to open the loaded file");
                    let _ = self
                        .outputs
                        .send(PlayerOutput::LoadFailed { file: *file })
                        .await;
                }
            }
            PlayerEvent::Exited { clean } => {
                return self.handle_player_death(clean).await;
            }
        }
        true
    }

    async fn handle_pause_observation(&mut self, paused: bool) {
        // Settle the estimate BEFORE flipping believed_pause — otherwise a
        // user unpause re-reads the new (playing) state and extrapolates the
        // whole paused interval as phantom playback, jumping the broadcast
        // position forward by the pause duration. Mirrors apply_desired_pause
        // / set_speed.
        self.reanchor_estimate();
        self.believed_pause = Some(paused);
        if self.pending_pause_echoes.front() == Some(&paused) {
            self.pending_pause_echoes.pop_front();
            return;
        }
        // Not what we commanded: the user did this. (A stale echo queue
        // self-heals — the main loop re-derives and re-commands.)
        self.pending_pause_echoes.clear();
        let output = if paused {
            tracing::info!("user paused");
            PlayerOutput::UserPaused
        } else {
            tracing::info!("user unpaused");
            PlayerOutput::UserUnpaused
        };
        let _ = self.outputs.send(output).await;
    }

    async fn flush_user_seek(&mut self) {
        if let Some((from_millis, to_millis, _)) = self.pending_user_seek.take() {
            tracing::info!(from_millis, to_millis, "user seek (reporting)");
            let _ = self
                .outputs
                .send(PlayerOutput::UserSeeked {
                    from_millis,
                    to_millis,
                })
                .await;
        }
    }

    async fn maybe_emit_position(&mut self) {
        // Ticks speak about `current`; never emit while the player hasn't
        // confirmed it holds that file (mid-load, or a user-loaded file is
        // up) — the estimate could otherwise extrapolate a stale anchor
        // under the wrong file's identity.
        if !self.player_on_current_file() {
            return;
        }
        let Some(position_millis) = self.estimate_now() else {
            return;
        };
        let Some(file) = self.current.as_ref().map(|(f, ..)| *f) else {
            return; // no loaded file to attribute the position to
        };
        let cadence = if self.believed_pause == Some(false) {
            POSITION_CADENCE_PLAYING
        } else {
            POSITION_CADENCE_PAUSED
        };
        let due = self
            .last_position_emit
            .is_none_or(|at| at.elapsed() >= cadence);
        if due {
            self.last_position_emit = Some(Instant::now());
            let _ = self
                .outputs
                .send(PlayerOutput::PositionTick {
                    file,
                    position_millis,
                })
                .await;
        }
    }

    /// The player went away. In attach mode this is a transient detach to
    /// wait out; in spawn mode it is a crash to escalate. Returns false
    /// when the actor should exit (spawn-mode relaunch impossible).
    async fn handle_player_death(&mut self, clean: bool) -> bool {
        // The player is gone either way: drop it and the echo bookkeeping.
        // No player, no evidence — a relaunched one re-announces its path.
        self.player = None;
        self.observed_path = None;
        self.pending_pause_echoes.clear();
        self.pending_seek_echoes = 0;
        self.pending_user_seek = None;
        self.believed_pause = None;

        if self.attach_mode {
            // Attach mode: the user closed/restarted their own mpv (the
            // socket closed). This is not a crash — never count it, never
            // pause the group; just wait for it to come back. The run loop
            // drives the retry via `reattach_at`.
            tracing::info!("attached mpv detached; waiting for it to return");
            self.begin_reattach();
            return true;
        }

        self.handle_crash(clean).await
    }

    /// Enter the attach-mode "waiting to re-attach" state: remember the
    /// resume position, drop volatile playback state, and schedule the
    /// first re-attach attempt now. [`Self::try_reattach`] takes it from
    /// there.
    fn begin_reattach(&mut self) {
        self.restore_millis = self.estimate_now();
        self.speed = 1.0;
        self.drift.reset();
        self.estimate = None;
        self.reattach_backoff = REATTACH_BACKOFF_INITIAL;
        self.reattach_at = Some(Instant::now());
    }

    /// Attach mode: one attempt to re-attach to the user's mpv. On success
    /// reloads the current file (position/pause restored on `Loaded`); on
    /// failure reschedules with capped backoff so the actor keeps waiting
    /// indefinitely instead of giving up.
    async fn try_reattach(&mut self) {
        self.reattach_at = None;
        // Attach mode never offloads spawns, so the factory is always here.
        let Some(factory) = self.factory.as_mut() else {
            return;
        };
        // Bound the probe so a down socket cannot park the run loop for the
        // full `wait_for_socket` deadline (see `REATTACH_PROBE_TIMEOUT`).
        let spawned = tokio::time::timeout(REATTACH_PROBE_TIMEOUT, factory.spawn()).await;
        if let Ok(Ok(player)) = spawned {
            tracing::info!("re-attached to mpv");
            self.reattach_backoff = REATTACH_BACKOFF_INITIAL;
            self.player = Some(player);
            self.reapply_overlays().await;
            if let Some((_, path, title)) = self.current.clone()
                && let Some(player) = &self.player
                && let Err(e) = player.load(&path, title.as_deref()).await
            {
                tracing::warn!("reload after re-attach failed: {e}");
            }
            // Position and pause state are restored on Loaded.
            return;
        }
        match spawned {
            Ok(Err(e)) => tracing::debug!(
                "attached mpv still down ({e}); retrying in {:?}",
                self.reattach_backoff
            ),
            // Err(_) = the probe itself timed out (socket never came up).
            _ => tracing::debug!(
                "attached mpv probe timed out; retrying in {:?}",
                self.reattach_backoff
            ),
        }
        self.reattach_at = Some(Instant::now() + self.reattach_backoff);
        self.reattach_backoff = (self.reattach_backoff * 2).min(REATTACH_BACKOFF_MAX);
    }

    /// Spawn mode: a player we own died. Escalate per design.md — silent
    /// relaunch, a global pause + chat notice on the second death within
    /// [`CRASH_FATAL_WINDOW`], and give up on the third. The relaunch
    /// itself runs in a background task ([`Self::begin_spawn`]); a
    /// failed relaunch exits the actor when its result lands. Returns
    /// false when the actor should exit.
    async fn handle_crash(&mut self, clean: bool) -> bool {
        // A clean exit is still unexpected (a deliberate quit goes
        // through Shutdown, which exits before this runs) — the user
        // closed mpv but the session needs a player, so relaunch either
        // way; the fatal window stops a crash loop.
        if clean {
            tracing::info!("player exited; relaunching");
        } else {
            tracing::warn!("player crashed; relaunching");
        }

        let now = Instant::now();
        let recent = self
            .last_death
            .is_some_and(|at| now.duration_since(at) < CRASH_FATAL_WINDOW);
        self.consecutive_crashes = if recent {
            self.consecutive_crashes + 1
        } else {
            1
        };
        self.last_death = Some(now);

        if self.consecutive_crashes == 2 {
            // Twice in quick succession: tell the session (global pause
            // + chat notice). The relaunch below then comes up paused.
            tracing::error!("player died twice within {CRASH_FATAL_WINDOW:?}");
            let _ = self.outputs.send(PlayerOutput::FatalCrash).await;
        }
        if self.consecutive_crashes >= CRASH_GIVE_UP_COUNT {
            // A crash loop: relaunching just feeds the fire (and spams the
            // log). Stop, but stay alive — loading a different file resets
            // the counter and brings a player back (see PlayerCommand::Load).
            tracing::error!(
                "player crashed {} times in a row; not relaunching until a new file is selected",
                self.consecutive_crashes
            );
            let _ = self.outputs.send(PlayerOutput::GaveUp).await;
            self.speed = 1.0;
            self.estimate = None;
            return true;
        }

        self.restore_millis = self.estimate_now();
        self.speed = 1.0;
        self.drift.reset();
        self.estimate = None;
        // Never awaited inline: mpv's socket wait can run for 30s, and
        // parking here would back the session's bounded player channel
        // up into its main loop (2026-08-12 review). The run loop picks
        // up the result and reloads `current`.
        self.begin_spawn(true);
        true
    }

    /// Spawn mode: launch a player in a background task — never inline,
    /// so the run loop keeps servicing commands for however long mpv
    /// takes to come up. No-op while a spawn is already in flight (its
    /// result covers this request too: [`Self::finish_spawn`] reloads
    /// whatever `current` is by then). `failure_exits` picks what a
    /// failed launch does: exit the actor (startup, crash relaunch) or
    /// stay alive awaiting another Load (the give-up recovery path).
    fn begin_spawn(&mut self, failure_exits: bool) {
        let Some(mut factory) = self.factory.take() else {
            return; // a spawn is already in flight
        };
        self.spawn_failure_exits = failure_exits;
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let result = factory.spawn().await;
            // If the actor exited meanwhile the receiver is gone and the
            // delivered player is dropped, which shuts it down via its
            // supervisor.
            let _ = tx.send((factory, result));
        });
        self.pending_spawn = Some(rx);
    }

    /// A background launch delivered its result: install the player and
    /// reload `current`, or escalate the failure. Returns false when the
    /// actor should exit.
    async fn finish_spawn(
        &mut self,
        delivered: Option<(F, Result<F::Player, PlayerError>)>,
    ) -> bool {
        self.pending_spawn = None;
        let Some((factory, result)) = delivered else {
            // The spawn task died without delivering (a panic in the
            // factory). The factory is lost with it, so no later launch
            // can succeed either.
            tracing::error!("player launch task vanished");
            let _ = self.outputs.send(PlayerOutput::FatalCrash).await;
            return false;
        };
        self.factory = Some(factory);
        match result {
            Ok(player) => {
                tracing::info!("player launched");
                self.player = Some(player);
                self.reapply_overlays().await;
                if let Some((file, path, title)) = self.current.clone()
                    && let Some(player) = &self.player
                    && let Err(e) = player.load(&path, title.as_deref()).await
                {
                    // Same contract as the Load handler's inline failure:
                    // tell the session so the file flips to Missing and
                    // re-resolves (the path may have gone stale while the
                    // player was down).
                    tracing::warn!(path = %path.display(), "load after launch failed: {e}");
                    let _ = self.outputs.send(PlayerOutput::LoadFailed { file }).await;
                }
                // Position and pause state are restored on Loaded.
                true
            }
            Err(e) => {
                tracing::error!("cannot launch the player: {e}");
                let _ = self.outputs.send(PlayerOutput::FatalCrash).await;
                !self.spawn_failure_exits
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use super::*;
    use crate::player::SpeakerName;
    use crate::player::mock::{MockCommand, MockControl, MockFactory, MockPlayer};

    const FILE: Ed2kHash = Ed2kHash([7; 16]);
    const FILE2: Ed2kHash = Ed2kHash([8; 16]);
    const BUDGET: Duration = Duration::from_secs(5);

    fn start(
        mocks: Vec<MockPlayer>,
        clock: Clock,
    ) -> (mpsc::Sender<PlayerCommand>, mpsc::Receiver<PlayerOutput>) {
        start_factory(MockFactory::new(mocks), clock)
    }

    fn start_factory(
        factory: MockFactory,
        clock: Clock,
    ) -> (mpsc::Sender<PlayerCommand>, mpsc::Receiver<PlayerOutput>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (out_tx, out_rx) = mpsc::channel(1024);
        tokio::spawn(run(factory, clock, cmd_rx, out_tx));
        (cmd_tx, out_rx)
    }

    fn fixed_clock(at: u64) -> Clock {
        Arc::new(move || at)
    }

    async fn expect_command(control: &mut MockControl) -> MockCommand {
        tokio::time::timeout(BUDGET, control.commands.recv())
            .await
            .expect("command budget exhausted")
            .expect("mock player dropped")
    }

    /// The next non-position output (position ticks flow constantly).
    async fn expect_output(outputs: &mut mpsc::Receiver<PlayerOutput>) -> PlayerOutput {
        loop {
            let out = tokio::time::timeout(BUDGET, outputs.recv())
                .await
                .expect("output budget exhausted")
                .expect("actor exited");
            if !matches!(out, PlayerOutput::PositionTick { .. }) {
                return out;
            }
        }
    }

    /// Everything emitted so far, position ticks filtered out.
    fn drain_outputs(outputs: &mut mpsc::Receiver<PlayerOutput>) -> Vec<PlayerOutput> {
        let mut out = Vec::new();
        while let Ok(o) = outputs.try_recv() {
            if !matches!(o, PlayerOutput::PositionTick { .. }) {
                out.push(o);
            }
        }
        out
    }

    /// Let the actor process whatever is in flight.
    async fn settle() {
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
    }

    /// Rig with one manual mock: file loaded, position known.
    async fn loaded_rig() -> (
        mpsc::Sender<PlayerCommand>,
        mpsc::Receiver<PlayerOutput>,
        MockControl,
    ) {
        let (player, mut control) = MockPlayer::pair();
        let (commands, outputs) = start(vec![player], fixed_clock(1_000_000));
        commands
            .send(PlayerCommand::Load {
                file: FILE,
                path: "/media/ep1.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        assert_eq!(
            expect_command(&mut control).await,
            MockCommand::Load("/media/ep1.mkv".into(), None)
        );
        // mpv's real order: the path echo confirms the load, then
        // file-loaded — attribution waits for the former.
        control
            .events
            .send(PlayerEvent::PathChanged {
                path: "/media/ep1.mkv".into(),
            })
            .unwrap();
        control.events.send(PlayerEvent::Loaded).unwrap();
        control
            .events
            .send(PlayerEvent::Position {
                position_millis: 10_000,
            })
            .unwrap();
        settle().await;
        (commands, outputs, control)
    }

    /// A bare [`Actor`] for unit-testing the position estimator directly,
    /// without driving the run loop. No player is spawned (the factory is
    /// empty); the returned receiver keeps the outputs channel alive.
    fn test_actor() -> (Actor<MockFactory>, mpsc::Receiver<PlayerOutput>) {
        let (out_tx, out_rx) = mpsc::channel(64);
        let actor = Actor {
            factory: Some(MockFactory::new(vec![])),
            outputs: out_tx,
            clock: fixed_clock(0),
            offset_millis: 0,
            player: None,
            current: None,
            observed_path: None,
            desired_playing: false,
            believed_pause: None,
            speed: 1.0,
            drift: DriftController::new(),
            pending_pause_echoes: VecDeque::new(),
            pending_seek_echoes: 0,
            pending_user_seek: None,
            restore_millis: None,
            estimate: None,
            last_position_emit: None,
            eof_reported: false,
            last_death: None,
            consecutive_crashes: 0,
            attach_mode: false,
            reattach_at: None,
            reattach_backoff: REATTACH_BACKOFF_INITIAL,
            pending_spawn: None,
            spawn_failure_exits: false,
            osd_chat: VecDeque::new(),
            blocker_overlay: None,
        };
        (actor, out_rx)
    }

    #[tokio::test(start_paused = true)]
    async fn load_failure_is_reported_upstream() {
        let (player, _control) = MockPlayer::pair_failing_load();
        let (commands, mut outputs) = start(vec![player], fixed_clock(1_000_000));
        commands
            .send(PlayerCommand::Load {
                file: FILE,
                path: "/gone/ep1.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        assert_eq!(
            expect_output(&mut outputs).await,
            PlayerOutput::LoadFailed { file: FILE }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_open_failure_after_a_successful_load_command_is_reported() {
        // The load *command* succeeded (mpv accepted loadfile — the rig is
        // fully loaded), so the command-error path never fires. The file
        // then fails to open: mpv emits end-file reason=error, surfaced as
        // PlayerEvent::LoadFailed. The actor must map it to the current
        // file and report it upstream so the session re-resolves a stale
        // path — the regression behind "unpaused on a file mpv never
        // loaded, showing only the forced title".
        let (_commands, mut outputs, control) = loaded_rig().await;
        let _ = drain_outputs(&mut outputs);
        control.events.send(PlayerEvent::LoadFailed).unwrap();
        settle().await;
        assert_eq!(
            drain_outputs(&mut outputs),
            vec![PlayerOutput::LoadFailed { file: FILE }]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn our_own_load_path_is_not_reported_but_a_dragged_path_is() {
        let (_commands, mut outputs, control) = loaded_rig().await;
        // Clear anything the rig left in flight.
        let _ = drain_outputs(&mut outputs);

        // mpv re-announces `path` after our own loadfile — the echo must be
        // swallowed (the actor compares against the path it commanded).
        control
            .events
            .send(PlayerEvent::PathChanged {
                path: "/media/ep1.mkv".into(),
            })
            .unwrap();
        settle().await;
        assert!(
            drain_outputs(&mut outputs).is_empty(),
            "our own load path must not be reported as a drag-in"
        );

        // A path we never commanded: the user dragged a file in.
        control
            .events
            .send(PlayerEvent::PathChanged {
                path: "/elsewhere/dragged.mkv".into(),
            })
            .unwrap();
        settle().await;
        assert_eq!(
            drain_outputs(&mut outputs),
            vec![PlayerOutput::PathObserved {
                path: "/elsewhere/dragged.mkv".into()
            }]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn programmatic_pause_flips_are_not_reported_as_user_actions() {
        let (commands, mut outputs, mut control) = loaded_rig().await;

        commands
            .send(PlayerCommand::SetPlaying(true))
            .await
            .unwrap();
        assert_eq!(
            expect_command(&mut control).await,
            MockCommand::SetPause(false)
        );
        control
            .events
            .send(PlayerEvent::PauseChanged(false))
            .unwrap();

        commands
            .send(PlayerCommand::SetPlaying(false))
            .await
            .unwrap();
        assert_eq!(
            expect_command(&mut control).await,
            MockCommand::SetPause(true)
        );
        control
            .events
            .send(PlayerEvent::PauseChanged(true))
            .unwrap();

        settle().await;
        assert_eq!(drain_outputs(&mut outputs), vec![]);
    }

    #[tokio::test(start_paused = true)]
    async fn set_playing_dedups_against_player_state() {
        let (commands, _outputs, mut control) = loaded_rig().await;
        // The file loads paused; asking for paused commands nothing.
        commands
            .send(PlayerCommand::SetPlaying(false))
            .await
            .unwrap();
        settle().await;
        assert_eq!(control.drain_commands(), vec![]);
    }

    #[tokio::test(start_paused = true)]
    async fn user_pause_is_reported_and_not_locally_corrected() {
        let (commands, mut outputs, mut control) = loaded_rig().await;
        commands
            .send(PlayerCommand::SetPlaying(true))
            .await
            .unwrap();
        assert_eq!(
            expect_command(&mut control).await,
            MockCommand::SetPause(false)
        );
        control
            .events
            .send(PlayerEvent::PauseChanged(false))
            .unwrap();

        // The user hits space. Reported upstream; the actor itself must
        // NOT correct it (the main loop decides, observe-and-correct).
        control
            .events
            .send(PlayerEvent::PauseChanged(true))
            .unwrap();
        assert_eq!(expect_output(&mut outputs).await, PlayerOutput::UserPaused);
        settle().await;
        assert_eq!(control.drain_commands(), vec![]);
    }

    #[tokio::test(start_paused = true)]
    async fn user_unpause_is_reported() {
        let (_commands, mut outputs, control) = loaded_rig().await;
        control
            .events
            .send(PlayerEvent::PauseChanged(false))
            .unwrap();
        assert_eq!(
            expect_output(&mut outputs).await,
            PlayerOutput::UserUnpaused
        );
    }

    #[tokio::test(start_paused = true)]
    async fn unpausing_in_mpv_after_a_long_pause_does_not_jump_the_broadcast_position() {
        // The file sits paused at 10_000 (loaded_rig's last Position sample).
        let (_commands, mut outputs, control) = loaded_rig().await;
        // A five-minute break passes with the player paused — mpv emits no
        // Position events while paused, so the anchor stays at 10_000.
        tokio::time::sleep(Duration::from_secs(300)).await;
        while outputs.try_recv().is_ok() {} // drop the paused-cadence ticks
        // The user unpauses directly in mpv (a flip we did not command).
        control
            .events
            .send(PlayerEvent::PauseChanged(false))
            .unwrap();
        assert_eq!(
            expect_output(&mut outputs).await,
            PlayerOutput::UserUnpaused
        );
        // The first position broadcast after the unpause must be near where
        // we paused (~10_000), NOT ~310_000 (the 5-minute pause counted as
        // phantom playback) — which would make this client a spurious
        // furthest-ahead drift leader and yank the group forward.
        let position = loop {
            let out = tokio::time::timeout(BUDGET, outputs.recv())
                .await
                .expect("position budget exhausted")
                .expect("actor exited");
            if let PlayerOutput::PositionTick {
                position_millis, ..
            } = out
            {
                break position_millis;
            }
        };
        assert!(
            position < 20_000,
            "unpause after a long pause broadcast position {position}; the \
             paused interval leaked into the estimate"
        );
    }

    proptest::proptest! {
        /// A user unpause (a pause flip we did not command) must never count
        /// the paused interval as playback. The estimate is anchored when the
        /// player pauses; after any paused interval, an unpause must re-anchor
        /// to that same position — extrapolation may only begin *from* the
        /// unpause, never reach back across the pause.
        ///
        /// Regression: `handle_pause_observation` set `believed_pause` before
        /// settling the estimate, so on an unpause `estimate_now` extrapolated
        /// the whole pause as phantom playback (the client could then broadcast
        /// a position far ahead and become a spurious drift leader).
        #[test]
        fn user_unpause_never_counts_the_paused_interval_as_playback(
            start_pos in 0u64..2_000_000,
            pause_millis in 0u64..3_600_000,
        ) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .start_paused(true)
                .build()
                .unwrap();
            let anchored = rt.block_on(async move {
                let (mut actor, _outputs) = test_actor();
                // The player is paused, anchored at `start_pos`.
                actor.believed_pause = Some(true);
                actor.note_position(start_pos);
                // A paused interval passes with no position events.
                tokio::time::sleep(Duration::from_millis(pause_millis)).await;
                // The user unpauses directly in the player.
                actor.handle_pause_observation(false).await;
                actor.estimate.as_ref().expect("estimate present").millis
            });
            proptest::prop_assert_eq!(
                anchored, start_pos,
                "unpause after a {}ms pause re-anchored at {} instead of {} \
                 (the paused interval leaked in as phantom playback)",
                pause_millis, anchored, start_pos,
            );
        }

        /// The mpv layer answers an observed pause with a paused-position
        /// query whose reply arrives as a Position event *after* the
        /// PauseChanged (player/mpv.rs). That reported position must win
        /// over whatever the estimate extrapolated while the pause
        /// observation was in flight (the 250ms EOF-disambiguation hold
        /// plus pipeline latency), and the estimate must stay frozen there
        /// for the whole pause — a Position-while-paused must never be
        /// ignored, or the overshoot silently returns.
        ///
        /// Regression (2026-07-20): Dagger paused mpv at 12.095s; dessplay
        /// broadcast 12.392 for the whole pause.
        #[test]
        fn player_reported_paused_position_overrides_the_extrapolated_estimate(
            anchor in 0u64..2_000_000,
            in_flight_millis in 0u64..2_000,
            reported_delta in 0u64..2_000,
            idle_millis in 0u64..3_600_000,
        ) {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .start_paused(true)
                .build()
                .unwrap();
            let reported = anchor + reported_delta;
            let frozen = rt.block_on(async move {
                let (mut actor, _outputs) = test_actor();
                // A file is loaded and confirmed (Position events are only
                // attributed once the player's path echo lands).
                actor.current = Some((FILE, "/media/ep1.mkv".into(), None));
                actor.observed_path = Some("/media/ep1.mkv".into());
                // Playing, last time-pos sample at `anchor`.
                actor.believed_pause = Some(false);
                actor.note_position(anchor);
                // The pause observation lands `in_flight_millis` after the
                // user actually paused; the estimate extrapolated on.
                tokio::time::sleep(Duration::from_millis(in_flight_millis)).await;
                actor.handle_player_event(Ok(PlayerEvent::PauseChanged(true))).await;
                // The paused-position reply: where mpv actually stopped.
                actor.handle_player_event(Ok(PlayerEvent::Position {
                    position_millis: reported,
                })).await;
                // Any amount of paused time later, the estimate still
                // reports the player's position, not the extrapolation.
                tokio::time::sleep(Duration::from_millis(idle_millis)).await;
                actor.estimate_now()
            });
            proptest::prop_assert_eq!(
                frozen, Some(reported),
                "paused estimate {:?} after a {}ms-late pause observation; \
                 expected the player-reported {}",
                frozen, in_flight_millis, reported,
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn scrubbing_debounces_into_one_user_seek() {
        let (_commands, mut outputs, control) = loaded_rig().await;
        control
            .events
            .send(PlayerEvent::Seeked {
                position_millis: 60_000,
            })
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1000)).await;
        assert_eq!(drain_outputs(&mut outputs), vec![], "debounce fired early");
        control
            .events
            .send(PlayerEvent::Seeked {
                position_millis: 90_000,
            })
            .unwrap();
        // 1499ms after the *last* seek: still pending.
        tokio::time::sleep(Duration::from_millis(1499)).await;
        assert_eq!(drain_outputs(&mut outputs), vec![], "debounce fired early");
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert_eq!(
            drain_outputs(&mut outputs),
            vec![PlayerOutput::UserSeeked {
                from_millis: 10_000,
                to_millis: 90_000,
            }],
            "scrubbing must coalesce to the final position"
        );
    }

    /// Regression: a loadfile must clear the commanded-seek echo counter.
    /// A programmatic seek (here a drift hard-seek) arms a pending seek
    /// echo; if a new file loads before that echo arrives, the stale
    /// counter would swallow the user's next real seek on the new file as
    /// if it were the echo. Before the fix, Load reset most state but left
    /// the echo counters (handle_player_death cleared them; Load didn't).
    #[tokio::test(start_paused = true)]
    async fn load_clears_a_pending_seek_echo() {
        let (commands, mut outputs, mut control) = loaded_rig().await;

        // A >3s drift hard-seek issues a programmatic seek and arms a
        // pending echo; the manual mock does not ack, so it stays
        // outstanding.
        sync_to_repeatedly(&commands, 20_000).await;
        assert_eq!(control.drain_commands(), vec![MockCommand::Seek(20_000)]);

        // A new file loads before that echo ever comes back.
        commands
            .send(PlayerCommand::Load {
                file: FILE2,
                path: "/media/ep2.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        assert_eq!(
            expect_command(&mut control).await,
            MockCommand::Load("/media/ep2.mkv".into(), None)
        );
        control
            .events
            .send(PlayerEvent::PathChanged {
                path: "/media/ep2.mkv".into(),
            })
            .unwrap();
        control.events.send(PlayerEvent::Loaded).unwrap();
        control
            .events
            .send(PlayerEvent::Position { position_millis: 0 })
            .unwrap();
        settle().await;
        let _ = drain_outputs(&mut outputs);

        // The user scrubs on the new file. With the stale echo cleared it
        // surfaces as a UserSeeked instead of being swallowed.
        control
            .events
            .send(PlayerEvent::Seeked {
                position_millis: 55_000,
            })
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1600)).await;
        assert_eq!(
            drain_outputs(&mut outputs),
            vec![PlayerOutput::UserSeeked {
                from_millis: 0,
                to_millis: 55_000,
            }],
            "the user's seek on the new file was swallowed as a stale echo"
        );
    }

    /// Regression (2026-07-27): `loadfile` is asynchronous, and on a slow
    /// machine (cold NAS, heavy mpv scripts) the player keeps emitting the
    /// *previous* file's observations — positions, seeks, even its EOF —
    /// long after the next episode's `Load` was commanded. Those trailing
    /// observations were attributed to the newly commanded file (the tag
    /// came from what we *commanded*, not from what the player *reported*),
    /// so a late-in-the-old-episode position was broadcast under the new
    /// file's identity: same-file leader election (which only moves the
    /// group forward) latched everyone onto it, and 85% of the new file's
    /// duration was falsely recorded watched. Attribution must wait for
    /// the player's own path echo.
    #[tokio::test(start_paused = true)]
    async fn slow_load_keeps_old_file_observations_off_the_new_file() {
        let (commands, mut outputs, control) = loaded_rig().await;
        // The old episode has played out to the 20-minute mark.
        control
            .events
            .send(PlayerEvent::Position {
                position_millis: 1_200_000,
            })
            .unwrap();
        settle().await;
        while outputs.try_recv().is_ok() {}

        // The next episode is commanded, but the slow player has not
        // switched yet (no path echo) — and keeps reporting the old file.
        commands
            .send(PlayerCommand::Load {
                file: FILE2,
                path: "/media/ep2.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        settle().await;
        control
            .events
            .send(PlayerEvent::Position {
                position_millis: 1_200_400,
            })
            .unwrap();
        control
            .events
            .send(PlayerEvent::Seeked {
                position_millis: 1_200_500,
            })
            .unwrap();
        control.events.send(PlayerEvent::Eof).unwrap();
        // Cross the paused position cadence and the seek debounce: any
        // misattributed output would surface in this window.
        tokio::time::sleep(SEEK_DEBOUNCE + Duration::from_secs(2)).await;
        let mut leaked = Vec::new();
        while let Ok(o) = outputs.try_recv() {
            leaked.push(o);
        }
        assert_eq!(
            leaked,
            vec![],
            "old-file observations were attributed to the new file"
        );

        // The slow load finally completes: mpv announces the new path,
        // then file-loaded, then the new file's first real position.
        control
            .events
            .send(PlayerEvent::PathChanged {
                path: "/media/ep2.mkv".into(),
            })
            .unwrap();
        control.events.send(PlayerEvent::Loaded).unwrap();
        control
            .events
            .send(PlayerEvent::Position { position_millis: 0 })
            .unwrap();
        settle().await;
        let tick = loop {
            let out = tokio::time::timeout(BUDGET, outputs.recv())
                .await
                .expect("no position tick after the load completed")
                .expect("actor exited");
            if let PlayerOutput::PositionTick {
                file,
                position_millis,
            } = out
            {
                break (file, position_millis);
            }
        };
        assert_eq!(
            tick,
            (FILE2, 0),
            "the new file's first tick must be its own position"
        );
    }

    /// Send the same authority sample enough times to clear the
    /// controller's engage debounce and re-command rate limit (one noisy
    /// sample must never trigger a correction, so the tests feed a
    /// sustained run).
    async fn sync_to_repeatedly(commands: &mpsc::Sender<PlayerCommand>, position_millis: u64) {
        for _ in 0..10 {
            commands
                .send(PlayerCommand::SyncTo {
                    position_millis,
                    timestamp: SharedTimestamp(1_000_000),
                    playing: false,
                })
                .await
                .unwrap();
        }
        settle().await;
    }

    #[tokio::test(start_paused = true)]
    async fn drift_below_engage_band_does_nothing() {
        let (commands, _outputs, mut control) = loaded_rig().await;
        sync_to_repeatedly(&commands, 10_000 + DRIFT_ENGAGE_MILLIS - 1).await;
        assert_eq!(control.drain_commands(), vec![]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_single_out_of_band_sample_does_not_slew() {
        let (commands, _outputs, mut control) = loaded_rig().await;
        // One noisy sample, then back in band: the engage debounce
        // must swallow it (each slew transition is an audible blip).
        commands
            .send(PlayerCommand::SyncTo {
                position_millis: 11_000,
                timestamp: SharedTimestamp(1_000_000),
                playing: false,
            })
            .await
            .unwrap();
        sync_to_repeatedly(&commands, 10_000).await;
        assert_eq!(control.drain_commands(), vec![]);
    }

    #[tokio::test(start_paused = true)]
    async fn drift_in_slew_band_slews_and_releases_on_convergence() {
        let (commands, _outputs, mut control) = loaded_rig().await;
        // 1s behind the authority (sustained): speed up, full slew.
        sync_to_repeatedly(&commands, 11_000).await;
        assert_eq!(
            control.drain_commands(),
            vec![MockCommand::SetSpeed(1.0 + SLEW_RATE)]
        );
        // Closing in: the slew tapers instead of stepping to 1.0.
        sync_to_repeatedly(&commands, 10_050).await;
        assert_eq!(control.drain_commands(), vec![MockCommand::SetSpeed(1.005)]);
        // Converged (under the release threshold): release the slew.
        sync_to_repeatedly(&commands, 10_010).await;
        assert_eq!(control.drain_commands(), vec![MockCommand::SetSpeed(1.0)]);
    }

    #[tokio::test(start_paused = true)]
    async fn drift_ahead_slews_down() {
        let (commands, _outputs, mut control) = loaded_rig().await;
        sync_to_repeatedly(&commands, 9_000).await;
        assert_eq!(
            control.drain_commands(),
            vec![MockCommand::SetSpeed(1.0 - SLEW_RATE)]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn drift_beyond_band_hard_seeks_and_suppresses_the_echo() {
        let (commands, mut outputs, mut control) = loaded_rig().await;
        sync_to_repeatedly(&commands, 20_000).await;
        assert_eq!(control.drain_commands(), vec![MockCommand::Seek(20_000)]);
        // The player echoes the seek; it must not become a user seek.
        control
            .events
            .send(PlayerEvent::Seeked {
                position_millis: 20_000,
            })
            .unwrap();
        tokio::time::sleep(SEEK_DEBOUNCE + Duration::from_millis(100)).await;
        assert_eq!(drain_outputs(&mut outputs), vec![]);
    }

    #[tokio::test(start_paused = true)]
    async fn playing_authority_sample_is_extrapolated_to_now() {
        let (commands, _outputs, mut control) = loaded_rig().await;
        // Sampled 5s ago at 10_000 while playing: the authority is at
        // ~15_000 now. We're at 10_000 → 5s behind → hard seek (after
        // the debounce run).
        for _ in 0..3 {
            commands
                .send(PlayerCommand::SyncTo {
                    position_millis: 10_000,
                    timestamp: SharedTimestamp(995_000),
                    playing: true,
                })
                .await
                .unwrap();
        }
        settle().await;
        assert_eq!(control.drain_commands(), vec![MockCommand::Seek(15_000)]);
    }

    /// Regression (2026-07-22): the bang-bang drift controller engaged
    /// and released at the same 100ms threshold, so a correction always
    /// ended parked at the edge of its own deadband — from where any
    /// sample wobble or slow clock-rate drift immediately re-crossed it,
    /// firing a fresh full-amplitude ±2% blip. Session logs showed
    /// hundreds of 100–300ms speed blips per hour, clearly audible as
    /// stutter even though a *sustained* 2% pitch-corrected slew is
    /// measurably transparent (verified against real mpv: a steady 1.02
    /// renders spectrally identical to baseline; the blip pattern puts
    /// broadband transients within 10dB of the signal).
    ///
    /// Closed loop: our extrapolated position advances at the commanded
    /// speed, the leader runs slightly fast, and every sample carries
    /// bounded noise. The controller must correct without flapping — the
    /// total number of speed *commands* stays small, and it must never
    /// command a speed outside the ±2% slew band.
    #[tokio::test(start_paused = true)]
    async fn noisy_leader_does_not_flap_the_playback_speed() {
        use rand::{Rng, SeedableRng, rngs::StdRng};

        let (commands, mut outputs, mut control) = loaded_rig().await;
        // Play, so the position estimate extrapolates (the loop closes:
        // a commanded slew actually moves our position).
        commands
            .send(PlayerCommand::SetPlaying(true))
            .await
            .unwrap();
        assert_eq!(
            expect_command(&mut control).await,
            MockCommand::SetPause(false)
        );
        control
            .events
            .send(PlayerEvent::PauseChanged(false))
            .unwrap();
        settle().await;
        let _ = control.drain_commands();

        // Reproducible from the seed alone (docs/testing-strategy.md).
        let mut rng = StdRng::seed_from_u64(0xDE55);
        // The rig anchors us at 10_000. The leader starts 200ms ahead
        // and runs 0.05% fast — the kind of rate mismatch two real
        // machines exhibit — and every sample wobbles by up to ±40ms
        // (network jitter + clock-offset error).
        let mut leader = 10_200.0f64;
        let mut speed_commands = 0u32;
        for _ in 0..3_000 {
            // 5 simulated minutes of 10Hz leader samples.
            tokio::time::sleep(Duration::from_millis(100)).await;
            leader += 100.0 * 1.0005;
            let noise: f64 = rng.random_range(-40.0..40.0);
            commands
                .send(PlayerCommand::SyncTo {
                    position_millis: (leader + noise) as u64,
                    timestamp: SharedTimestamp(1_000_000),
                    playing: true,
                })
                .await
                .unwrap();
            settle().await;
            for cmd in control.drain_commands() {
                if let MockCommand::SetSpeed(speed) = cmd {
                    speed_commands += 1;
                    assert!(
                        (1.0 - SLEW_RATE..=1.0 + SLEW_RATE).contains(&speed),
                        "commanded speed {speed} outside the slew band"
                    );
                }
            }
            while outputs.try_recv().is_ok() {}
        }

        assert!(
            speed_commands <= 40,
            "{speed_commands} speed commands in 5 simulated minutes — the \
             drift controller is flapping (each command is an audible \
             tempo step; a handful per correction episode is the budget)"
        );
    }

    /// Regression (2026-08-12 review): drift correction is file-attributed
    /// work like every other observation. While the player is off the
    /// current file — the user dragged their own file in (the normal
    /// workflow in attach mode), or a load is still in flight — authority
    /// samples must not slew or hard-seek the player: it would yank the
    /// user's own unrelated video around.
    #[tokio::test(start_paused = true)]
    async fn drift_correction_is_gated_while_the_player_is_off_the_current_file() {
        let (commands, mut outputs, mut control) = loaded_rig().await;
        // The user drags their own file into mpv: the gate closes.
        control
            .events
            .send(PlayerEvent::PathChanged {
                path: "/elsewhere/dragged.mkv".into(),
            })
            .unwrap();
        settle().await;
        let _ = drain_outputs(&mut outputs);
        // The authority is 10s away — normally a sustained hard seek.
        sync_to_repeatedly(&commands, 20_000).await;
        assert_eq!(
            control.drain_commands(),
            vec![],
            "drift correction acted on the user's own file"
        );
    }

    /// The controller's engage/seek run must not survive a gate closure:
    /// out-of-band samples accumulated before the drag-in plus one after
    /// the gate re-opens must not complete the run and fire a correction
    /// off the stale estimate — the run restarts clean.
    #[tokio::test(start_paused = true)]
    async fn a_gate_closure_resets_the_drift_run() {
        let (commands, mut outputs, mut control) = loaded_rig().await;
        // Two out-of-band samples: one short of the ENGAGE_RUN of 3.
        for _ in 0..2 {
            commands
                .send(PlayerCommand::SyncTo {
                    position_millis: 20_000,
                    timestamp: SharedTimestamp(1_000_000),
                    playing: false,
                })
                .await
                .unwrap();
        }
        settle().await;
        assert_eq!(control.drain_commands(), vec![]);

        // The gate closes (drag-in) and re-opens (back on our file).
        control
            .events
            .send(PlayerEvent::PathChanged {
                path: "/elsewhere/dragged.mkv".into(),
            })
            .unwrap();
        control
            .events
            .send(PlayerEvent::PathChanged {
                path: "/media/ep1.mkv".into(),
            })
            .unwrap();
        settle().await;
        let _ = drain_outputs(&mut outputs);

        // With the reset, further samples start a fresh run instead of
        // completing the stale one.
        for _ in 0..2 {
            commands
                .send(PlayerCommand::SyncTo {
                    position_millis: 20_000,
                    timestamp: SharedTimestamp(1_000_000),
                    playing: false,
                })
                .await
                .unwrap();
        }
        settle().await;
        assert_eq!(
            control.drain_commands(),
            vec![],
            "a stale pre-drag-in drift run fired a correction"
        );

        // Sanity: a full sustained run still corrects.
        sync_to_repeatedly(&commands, 20_000).await;
        assert_eq!(control.drain_commands(), vec![MockCommand::Seek(20_000)]);
    }

    /// Regression (2026-08-12 review): a programmatic seek's echo must be
    /// consumed even when it arrives while the attribution gate is closed
    /// — it is *our own* seek. Leaked, the stale counter swallows the
    /// user's next genuine seek after the gate re-opens.
    #[tokio::test(start_paused = true)]
    async fn a_gated_out_programmatic_seek_echo_is_still_consumed() {
        let (commands, mut outputs, mut control) = loaded_rig().await;
        // A drift hard seek arms a pending echo (the manual mock does
        // not ack, so it stays outstanding).
        sync_to_repeatedly(&commands, 20_000).await;
        assert_eq!(control.drain_commands(), vec![MockCommand::Seek(20_000)]);

        // The user drags in their own file before the echo lands…
        control
            .events
            .send(PlayerEvent::PathChanged {
                path: "/elsewhere/dragged.mkv".into(),
            })
            .unwrap();
        // …and the echo arrives while the gate is closed.
        control
            .events
            .send(PlayerEvent::Seeked {
                position_millis: 20_000,
            })
            .unwrap();
        settle().await;

        // Back on our file; the user scrubs. The scrub must surface as a
        // UserSeeked, not be swallowed as the leaked echo.
        control
            .events
            .send(PlayerEvent::PathChanged {
                path: "/media/ep1.mkv".into(),
            })
            .unwrap();
        control
            .events
            .send(PlayerEvent::Seeked {
                position_millis: 55_000,
            })
            .unwrap();
        tokio::time::sleep(SEEK_DEBOUNCE + Duration::from_millis(100)).await;
        let outs = drain_outputs(&mut outputs);
        assert!(
            outs.contains(&PlayerOutput::UserSeeked {
                from_millis: 20_000,
                to_millis: 55_000,
            }),
            "the user's scrub was swallowed by a leaked programmatic echo: {outs:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn drift_correction_pauses_while_the_user_scrubs() {
        let (commands, _outputs, mut control) = loaded_rig().await;
        control
            .events
            .send(PlayerEvent::Seeked {
                position_millis: 60_000,
            })
            .unwrap();
        settle().await;
        // Authority data arriving mid-scrub must not fight the user.
        commands
            .send(PlayerCommand::SyncTo {
                position_millis: 10_000,
                timestamp: SharedTimestamp(1_000_000),
                playing: false,
            })
            .await
            .unwrap();
        settle().await;
        assert_eq!(control.drain_commands(), vec![]);
    }

    #[tokio::test(start_paused = true)]
    async fn position_cadence_is_dense_playing_sparse_paused() {
        let (commands, mut outputs, control) = loaded_rig().await;
        // Paused: 1s cadence.
        tokio::time::sleep(Duration::from_secs(3)).await;
        let paused_ticks = {
            let mut n = 0;
            while let Ok(o) = outputs.try_recv() {
                if matches!(o, PlayerOutput::PositionTick { .. }) {
                    n += 1;
                }
            }
            n
        };
        assert!(
            (3..=4).contains(&paused_ticks),
            "paused cadence should be ~1s: got {paused_ticks} ticks in 3s"
        );

        commands
            .send(PlayerCommand::SetPlaying(true))
            .await
            .unwrap();
        control
            .events
            .send(PlayerEvent::PauseChanged(false))
            .unwrap();
        settle().await;
        while outputs.try_recv().is_ok() {}
        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut playing_positions = Vec::new();
        while let Ok(o) = outputs.try_recv() {
            if let PlayerOutput::PositionTick {
                position_millis, ..
            } = o
            {
                playing_positions.push(position_millis);
            }
        }
        assert!(
            (9..=11).contains(&playing_positions.len()),
            "playing cadence should be ~100ms: got {} ticks in 1s",
            playing_positions.len()
        );
        // And the position must advance between ticks (extrapolation —
        // the mock sent no further Position events).
        assert!(
            playing_positions.last() > playing_positions.first(),
            "extrapolated position must advance while playing"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn eof_reported_once_until_a_seek_resets_it() {
        let (_commands, mut outputs, control) = loaded_rig().await;
        control.events.send(PlayerEvent::Eof).unwrap();
        assert_eq!(
            expect_output(&mut outputs).await,
            PlayerOutput::Eof { file: FILE }
        );
        control.events.send(PlayerEvent::Eof).unwrap();
        settle().await;
        assert_eq!(drain_outputs(&mut outputs), vec![], "duplicate EOF leaked");

        // The user rewinds (seek resets the latch), watches to the end
        // again: a fresh EOF must be reported.
        control
            .events
            .send(PlayerEvent::Seeked { position_millis: 0 })
            .unwrap();
        tokio::time::sleep(SEEK_DEBOUNCE + Duration::from_millis(100)).await;
        while outputs.try_recv().is_ok() {}
        control.events.send(PlayerEvent::Eof).unwrap();
        assert_eq!(
            expect_output(&mut outputs).await,
            PlayerOutput::Eof { file: FILE }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn crash_relaunches_and_restores_file_position_and_pause() {
        let (p1, mut c1) = MockPlayer::pair();
        let (p2, mut c2) = MockPlayer::pair();
        let (commands, mut outputs) = start(vec![p1, p2], fixed_clock(0));
        commands
            .send(PlayerCommand::Load {
                file: FILE,
                path: "/media/ep1.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        expect_command(&mut c1).await;
        c1.events
            .send(PlayerEvent::PathChanged {
                path: "/media/ep1.mkv".into(),
            })
            .unwrap();
        c1.events.send(PlayerEvent::Loaded).unwrap();
        c1.events
            .send(PlayerEvent::Position {
                position_millis: 42_000,
            })
            .unwrap();
        commands
            .send(PlayerCommand::SetPlaying(true))
            .await
            .unwrap();
        assert_eq!(expect_command(&mut c1).await, MockCommand::SetPause(false));
        c1.events.send(PlayerEvent::PauseChanged(false)).unwrap();
        settle().await;

        c1.events
            .send(PlayerEvent::Exited { clean: false })
            .unwrap();
        assert_eq!(
            expect_command(&mut c2).await,
            MockCommand::Load("/media/ep1.mkv".into(), None),
            "relaunch must reload the current file"
        );
        c2.events.send(PlayerEvent::Loaded).unwrap();
        assert_eq!(
            expect_command(&mut c2).await,
            MockCommand::Seek(42_000),
            "relaunch must restore the position"
        );
        assert_eq!(
            expect_command(&mut c2).await,
            MockCommand::SetPause(false),
            "relaunch must restore the desired pause state"
        );
        settle().await;
        assert_eq!(
            drain_outputs(&mut outputs),
            vec![],
            "a single crash is not fatal"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn second_crash_within_window_is_fatal() {
        let (p1, c1) = MockPlayer::pair();
        let (p2, mut c2) = MockPlayer::pair();
        let (p3, mut c3) = MockPlayer::pair();
        let (commands, mut outputs) = start(vec![p1, p2, p3], fixed_clock(0));
        commands
            .send(PlayerCommand::Load {
                file: FILE,
                path: "/media/ep1.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        settle().await;

        c1.events
            .send(PlayerEvent::Exited { clean: false })
            .unwrap();
        expect_command(&mut c2).await; // reload on the second instance
        tokio::time::sleep(Duration::from_secs(5)).await;
        while outputs.try_recv().is_ok() {}

        c2.events
            .send(PlayerEvent::Exited { clean: false })
            .unwrap();
        assert_eq!(expect_output(&mut outputs).await, PlayerOutput::FatalCrash);
        // Still relaunched — the fatal signal pauses the session, and a
        // paused player is the safe state to come back in.
        expect_command(&mut c3).await;
    }

    #[tokio::test(start_paused = true)]
    async fn third_crash_within_window_gives_up_then_recovers_on_new_file() {
        const FILE2: Ed2kHash = Ed2kHash([9; 16]);
        let (p1, c1) = MockPlayer::pair();
        let (p2, c2) = MockPlayer::pair();
        let (p3, c3) = MockPlayer::pair();
        let (p4, mut c4) = MockPlayer::pair();
        let (commands, mut outputs) = start(vec![p1, p2, p3, p4], fixed_clock(0));
        commands
            .send(PlayerCommand::Load {
                file: FILE,
                path: "/media/ep1.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        settle().await;

        // Crash 1: silent relaunch onto p2.
        c1.events
            .send(PlayerEvent::Exited { clean: false })
            .unwrap();
        settle().await;
        // Crash 2: FatalCrash, relaunch onto p3.
        c2.events
            .send(PlayerEvent::Exited { clean: false })
            .unwrap();
        assert_eq!(expect_output(&mut outputs).await, PlayerOutput::FatalCrash);
        settle().await;
        // Crash 3: give up — no relaunch.
        c3.events
            .send(PlayerEvent::Exited { clean: false })
            .unwrap();
        assert_eq!(expect_output(&mut outputs).await, PlayerOutput::GaveUp);
        settle().await;
        assert!(
            c4.commands.try_recv().is_err(),
            "after giving up, the actor must not spawn another player"
        );

        // A different file is the recovery trigger: spawn p4 and load it.
        commands
            .send(PlayerCommand::Load {
                file: FILE2,
                path: "/media/ep2.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        assert_eq!(
            expect_command(&mut c4).await,
            MockCommand::Load("/media/ep2.mkv".into(), None),
            "loading a new file recovers from the crash-loop give-up"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn crashes_outside_the_window_are_not_fatal() {
        let (p1, c1) = MockPlayer::pair();
        let (p2, c2) = MockPlayer::pair();
        let (p3, mut c3) = MockPlayer::pair();
        let (commands, mut outputs) = start(vec![p1, p2, p3], fixed_clock(0));
        commands
            .send(PlayerCommand::Load {
                file: FILE,
                path: "/media/ep1.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        settle().await;

        c1.events
            .send(PlayerEvent::Exited { clean: false })
            .unwrap();
        settle().await;
        tokio::time::sleep(CRASH_FATAL_WINDOW + Duration::from_secs(1)).await;
        c2.events
            .send(PlayerEvent::Exited { clean: false })
            .unwrap();
        expect_command(&mut c3).await;
        settle().await;
        assert_eq!(drain_outputs(&mut outputs), vec![]);
    }

    #[tokio::test(start_paused = true)]
    async fn attach_mode_socket_close_is_a_transient_detach_not_a_crash() {
        // In attach mode the user owns mpv; closing/restarting it closes the
        // socket -> Exited{clean:true}. That is a transient *detach*, not a
        // crash: it must never be counted toward the crash-escalation, so two
        // quick restarts do NOT fire FatalCrash (a *synced* "my player crashed
        // — pausing" chat that pauses the whole group), and three do NOT give
        // up. The actor simply re-attaches each time.
        let (p1, c1) = MockPlayer::pair();
        let (p2, mut c2) = MockPlayer::pair();
        let (p3, mut c3) = MockPlayer::pair();
        let (commands, mut outputs) =
            start_factory(MockFactory::attach([p1, p2, p3]), fixed_clock(0));
        commands
            .send(PlayerCommand::Load {
                file: FILE,
                path: "/media/ep1.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        settle().await;

        // The user restarts their mpv: socket closes.
        c1.events.send(PlayerEvent::Exited { clean: true }).unwrap();
        assert_eq!(
            expect_command(&mut c2).await,
            MockCommand::Load("/media/ep1.mkv".into(), None),
            "re-attach must reload the current file"
        );
        settle().await;

        // And again, immediately (well within CRASH_FATAL_WINDOW).
        c2.events.send(PlayerEvent::Exited { clean: true }).unwrap();
        assert_eq!(
            expect_command(&mut c3).await,
            MockCommand::Load("/media/ep1.mkv".into(), None),
            "re-attach must reload the current file on the second detach too"
        );
        settle().await;

        assert_eq!(
            drain_outputs(&mut outputs),
            vec![],
            "attach-mode detaches must not escalate to FatalCrash/GaveUp"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn attach_mode_waits_indefinitely_for_mpv_to_return() {
        // After a detach the user's mpv stays down far longer than the 10s
        // socket-wait deadline. The actor must keep retrying (capped backoff),
        // never emit FatalCrash, never exit — and re-attach + reload once mpv
        // comes back.
        let (p1, c1) = MockPlayer::pair();
        let (p2, mut c2) = MockPlayer::pair();
        let factory = MockFactory::attach([p1])
            .then_down()
            .then_down()
            .then_down()
            .then_up(p2);
        let (commands, mut outputs) = start_factory(factory, fixed_clock(0));
        commands
            .send(PlayerCommand::Load {
                file: FILE,
                path: "/media/ep1.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        settle().await;

        // mpv goes away and stays down across several retry attempts.
        c1.events.send(PlayerEvent::Exited { clean: true }).unwrap();
        // Far past the SOCKET_WAIT deadline.
        tokio::time::sleep(Duration::from_secs(60)).await;

        assert_eq!(
            drain_outputs(&mut outputs),
            vec![],
            "a down attached mpv must not escalate to a crash or give-up"
        );
        // mpv returned on a later retry: the actor re-attached and reloaded.
        assert_eq!(
            expect_command(&mut c2).await,
            MockCommand::Load("/media/ep1.mkv".into(), None),
            "the actor must re-attach and restore the file once mpv returns"
        );
    }

    /// Regression: a re-attach probe that hangs (a down socket in
    /// `wait_for_socket`) must not park the run loop. A Shutdown issued while
    /// the probe is in flight has to be serviced — the probe is bounded by
    /// `REATTACH_PROBE_TIMEOUT`. Unbounded, the hung spawn awaited inline in
    /// the `select!` arm would block every other command indefinitely.
    #[tokio::test(start_paused = true)]
    async fn a_hung_reattach_probe_does_not_block_shutdown() {
        let (p1, c1) = MockPlayer::pair();
        // Initial attach uses p1; the first re-attach probe hangs forever.
        let factory = MockFactory::attach([p1]).then_hang();
        let (commands, mut outputs) = start_factory(factory, fixed_clock(0));
        commands
            .send(PlayerCommand::Load {
                file: FILE,
                path: "/media/ep1.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        settle().await;

        // mpv detaches; the actor begins re-attaching and the next probe hangs.
        c1.events.send(PlayerEvent::Exited { clean: true }).unwrap();
        settle().await;

        // Shutdown arrives while the probe is hung. It must be serviced: the
        // actor exits and its outputs channel closes. Unbounded, the hung
        // probe would park the select loop and this would time out.
        commands.send(PlayerCommand::Shutdown).await.unwrap();
        let exited = tokio::time::timeout(Duration::from_secs(30), async {
            while outputs.recv().await.is_some() {}
        })
        .await;
        assert!(
            exited.is_ok(),
            "Shutdown was not serviced while a re-attach probe hung"
        );
    }

    /// Regression (2026-08-12 review): a spawn-mode relaunch whose
    /// `factory.spawn()` hangs (mpv's `wait_for_socket` can legitimately
    /// run for its full 30s deadline — a slow startup is not a crash)
    /// must not park the run loop. A Shutdown issued while the relaunch
    /// is in flight has to be serviced — the spawn runs in its own task,
    /// never awaited inline in a `select!` arm. Mirrors
    /// `a_hung_reattach_probe_does_not_block_shutdown`, which covers
    /// attach mode only.
    #[tokio::test(start_paused = true)]
    async fn a_hung_relaunch_spawn_does_not_block_shutdown() {
        let (p1, c1) = MockPlayer::pair();
        // Initial launch uses p1; the post-crash relaunch hangs forever.
        let factory = MockFactory::new([p1]).then_hang();
        let (commands, mut outputs) = start_factory(factory, fixed_clock(0));
        commands
            .send(PlayerCommand::Load {
                file: FILE,
                path: "/media/ep1.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        settle().await;

        // The player crashes; the relaunch spawn hangs.
        c1.events
            .send(PlayerEvent::Exited { clean: false })
            .unwrap();
        settle().await;

        // Shutdown arrives while the relaunch hangs. It must be serviced:
        // the actor exits and its outputs channel closes. Awaited inline,
        // the hung spawn would park the select loop and this would time
        // out.
        commands.send(PlayerCommand::Shutdown).await.unwrap();
        let exited = tokio::time::timeout(Duration::from_secs(30), async {
            while outputs.recv().await.is_some() {}
        })
        .await;
        assert!(
            exited.is_ok(),
            "Shutdown was not serviced while a relaunch spawn hung"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn duration_and_subtitles_are_forwarded() {
        let (_commands, mut outputs, control) = loaded_rig().await;
        control
            .events
            .send(PlayerEvent::DurationKnown {
                duration_millis: 1_440_000,
            })
            .unwrap();
        assert_eq!(
            expect_output(&mut outputs).await,
            PlayerOutput::DurationKnown {
                file: FILE,
                duration_millis: 1_440_000
            }
        );
        control
            .events
            .send(PlayerEvent::SubtitleLine {
                text: "こんにちは".into(),
                speaker: SpeakerName::new("Frieren"),
            })
            .unwrap();
        // The in-video position is attached from the actor's estimate;
        // loaded_rig parked it at 10_000 (paused, so it doesn't advance).
        // The speaker rides along unchanged.
        assert_eq!(
            expect_output(&mut outputs).await,
            PlayerOutput::SubtitleLine {
                text: "こんにちは".into(),
                speaker: SpeakerName::new("Frieren"),
                position_millis: 10_000,
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn subtitle_position_is_zero_before_any_sample() {
        // No file loaded, so estimate_now() is None -> 0 (honest 00:00).
        let (player, control) = MockPlayer::pair();
        let (_commands, mut outputs) = start(vec![player], fixed_clock(1_000_000));
        control
            .events
            .send(PlayerEvent::SubtitleLine {
                text: "hi".into(),
                speaker: None,
            })
            .unwrap();
        assert_eq!(
            expect_output(&mut outputs).await,
            PlayerOutput::SubtitleLine {
                text: "hi".into(),
                speaker: None,
                position_millis: 0,
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn screenshot_command_reaches_the_player_with_its_path() {
        let (commands, _outputs, mut control) = loaded_rig().await;
        let path = PathBuf::from("/tmp/marquee-test.png");
        commands
            .send(PlayerCommand::Screenshot(path.clone()))
            .await
            .unwrap();
        assert_eq!(
            expect_command(&mut control).await,
            MockCommand::Screenshot(path)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn osd_chat_rolls_up_and_expires_individually() {
        let (commands, _outputs, mut control) = loaded_rig().await;
        commands
            .send(PlayerCommand::ShowOsd("Baughn: hello".into()))
            .await
            .unwrap();
        assert_eq!(
            expect_command(&mut control).await,
            MockCommand::SetOsdOverlay(1, Some("{\\an7\\fs26}Baughn: hello".into()))
        );
        // A second message joins the first instead of replacing it (#16).
        tokio::time::advance(Duration::from_secs(2)).await;
        commands
            .send(PlayerCommand::ShowOsd("Nero: hi".into()))
            .await
            .unwrap();
        assert_eq!(
            expect_command(&mut control).await,
            MockCommand::SetOsdOverlay(1, Some("{\\an7\\fs26}Baughn: hello\\NNero: hi".into()))
        );
        // The first expires alone at its own 8s mark…
        tokio::time::advance(
            OSD_CHAT_RETENTION - Duration::from_secs(2) + Duration::from_millis(1),
        )
        .await;
        assert_eq!(
            expect_command(&mut control).await,
            MockCommand::SetOsdOverlay(1, Some("{\\an7\\fs26}Nero: hi".into()))
        );
        // …and the second's expiry clears the overlay.
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(
            expect_command(&mut control).await,
            MockCommand::SetOsdOverlay(1, None)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn blocker_overlay_sets_clears_and_survives_relaunch() {
        let (first, mut control) = MockPlayer::pair();
        let (second, mut control2) = MockPlayer::pair();
        let (commands, _outputs) = start(vec![first, second], fixed_clock(1_000_000));
        commands
            .send(PlayerCommand::Load {
                file: FILE,
                path: "/media/ep1.mkv".into(),
                title: None,
            })
            .await
            .unwrap();
        control.events.send(PlayerEvent::Loaded).unwrap();
        settle().await;
        commands
            .send(PlayerCommand::SetBlockerOverlay(Some(
                "Waiting for kim (paused)".into(),
            )))
            .await
            .unwrap();
        let sent = drain_until(&mut control, |cmd| {
            matches!(cmd, MockCommand::SetOsdOverlay(2, Some(_)))
        })
        .await;
        assert_eq!(
            sent,
            MockCommand::SetOsdOverlay(2, Some("{\\an9\\fs26}Waiting for kim (paused)".into()))
        );

        // Crash: the relaunched player gets the overlay re-applied — a
        // fresh mpv process starts with clean overlay slots. (A first
        // death relaunches silently, so no output is expected here.)
        control
            .events
            .send(PlayerEvent::Exited { clean: false })
            .unwrap();
        let reapplied = drain_until(&mut control2, |cmd| {
            matches!(cmd, MockCommand::SetOsdOverlay(2, Some(_)))
        })
        .await;
        assert_eq!(
            reapplied,
            MockCommand::SetOsdOverlay(2, Some("{\\an9\\fs26}Waiting for kim (paused)".into()))
        );

        // Clearing propagates as a removal.
        commands
            .send(PlayerCommand::SetBlockerOverlay(None))
            .await
            .unwrap();
        let cleared = drain_until(&mut control2, |cmd| {
            matches!(cmd, MockCommand::SetOsdOverlay(2, None))
        })
        .await;
        assert_eq!(cleared, MockCommand::SetOsdOverlay(2, None));
    }

    /// Pump commands (advancing paused time) until one matches.
    async fn drain_until(
        control: &mut MockControl,
        pred: impl Fn(&MockCommand) -> bool,
    ) -> MockCommand {
        let deadline = tokio::time::Instant::now() + BUDGET;
        loop {
            if let Some(cmd) = control.try_command() {
                if pred(&cmd) {
                    return cmd;
                }
                continue;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "no matching command within budget"
            );
            tokio::time::advance(Duration::from_millis(20)).await;
        }
    }
}
