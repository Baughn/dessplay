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
//!   position, in three bands: ignore below [`DRIFT_IGNORE_MILLIS`],
//!   slew by ±[`SLEW_RATE`] up to [`DRIFT_HARD_SEEK_MILLIS`], hard seek
//!   beyond.
//! - **Seek debounce.** User scrubbing is only reported
//!   [`SEEK_DEBOUNCE`] after the last seek; drift correction is
//!   suspended while a debounce is pending (the scrubber is about to
//!   become the authority).
//! - **Position cadence.** Emits [`PlayerOutput::PositionTick`] every
//!   100ms while playing, every 1s while paused.
//! - **Crash supervision.** A dead player is relaunched and restored
//!   (same file, last position, desired pause state). A second death
//!   within [`CRASH_FATAL_WINDOW`] additionally emits
//!   [`PlayerOutput::FatalCrash`], which the main loop turns into a
//!   global pause and a chat notice — the relaunch then comes up paused,
//!   which is exactly the safe state if the file itself is crashing the
//!   player.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use dessplay_core::types::{Ed2kHash, SharedTimestamp};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::network::Clock;
use crate::player::{Player, PlayerError, PlayerEvent, PlayerFactory};

/// Drift smaller than this is ignored.
pub const DRIFT_IGNORE_MILLIS: u64 = 100;
/// Drift larger than this is corrected with a hard seek; in between,
/// playback speed is slewed.
pub const DRIFT_HARD_SEEK_MILLIS: u64 = 3_000;
/// Maximum slew, as a speed delta (±2%; pitch-corrected and invisible).
pub const SLEW_RATE: f64 = 0.02;
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
/// Attach mode: first delay before re-probing the user's mpv socket after
/// it goes away (it usually comes back quickly).
pub const REATTACH_BACKOFF_INITIAL: Duration = Duration::from_millis(500);
/// Attach mode: the re-attach backoff never grows past this.
pub const REATTACH_BACKOFF_MAX: Duration = Duration::from_secs(10);

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
    /// Updated shared-clock offset (server minus local), from time sync.
    ClockOffset(i64),
    /// Display a message on the video.
    ShowOsd(String),
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
        /// Position landed on, milliseconds.
        position_millis: u64,
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
        speaker: Option<String>,
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
    factory: F,
    outputs: mpsc::Sender<PlayerOutput>,
    clock: Clock,
    offset_millis: i64,
    player: Option<F::Player>,
    /// Now-loaded file, path, and display title (for EOF attribution and
    /// relaunch — the title must be re-applied so a relaunched player still
    /// shows the real filename for hash-named cache files).
    current: Option<(Ed2kHash, PathBuf, Option<String>)>,
    /// The group's desired playback state.
    desired_playing: bool,
    /// Pause state we believe the player is in (last command or
    /// observation); `None` until something is known.
    believed_pause: Option<bool>,
    /// Current slew speed (1.0 = not slewing).
    speed: f64,
    /// Pause flips we commanded and haven't seen echoed yet.
    pending_pause_echoes: VecDeque<bool>,
    /// Seeks we commanded and haven't seen echoed yet.
    pending_seek_echoes: usize,
    /// A user seek waiting out the debounce window.
    pending_user_seek: Option<(u64, Instant)>,
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
        factory,
        outputs,
        clock,
        offset_millis: 0,
        player: None,
        current: None,
        desired_playing: false,
        believed_pause: None,
        speed: 1.0,
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
    };

    match actor.factory.spawn().await {
        Ok(player) => {
            actor.player = Some(player);
            tracing::info!("player launched");
        }
        // Attach mode: the user's mpv isn't up yet. That is not fatal —
        // wait for it to come up rather than pausing the group and exiting
        // (design.md: attach mode waits for mpv to come back).
        Err(e) if actor.attach_mode => {
            tracing::info!("attached mpv not up yet ({e}); waiting for it to appear");
            actor.begin_reattach();
        }
        Err(e) => {
            tracing::error!("cannot launch the player: {e}");
            let _ = actor.outputs.send(PlayerOutput::FatalCrash).await;
            return;
        }
    }

    let mut cadence = tokio::time::interval(POSITION_CADENCE_PLAYING);
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let debounce_at = actor.pending_user_seek.map(|(_, at)| at + SEEK_DEBOUNCE);
        let reattach_at = actor.reattach_at;
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
                self.set_speed(1.0).await;
                if different {
                    // A new file is a clean slate for the crash-loop counter.
                    self.consecutive_crashes = 0;
                    self.last_death = None;
                    if self.player.is_none() && self.reattach_at.is_none() {
                        // We gave up relaunching after a crash loop; a new
                        // file is the recovery trigger — bring a player back.
                        // (Skipped while a re-attach is already scheduled: the
                        // pending retry will pick up this new `current`.)
                        match self.factory.spawn().await {
                            Ok(player) => {
                                tracing::info!("relaunching player for the new file");
                                self.player = Some(player);
                            }
                            Err(e) => {
                                tracing::error!("could not relaunch the player: {e}");
                                let _ = self.outputs.send(PlayerOutput::FatalCrash).await;
                            }
                        }
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
            PlayerCommand::ClockOffset(offset_millis) => {
                self.offset_millis = offset_millis;
            }
            PlayerCommand::ShowOsd(text) => {
                if let Some(player) = &self.player
                    && let Err(e) = player.show_osd(&text).await
                {
                    tracing::debug!("osd failed: {e}");
                }
            }
            PlayerCommand::Shutdown => {}
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
        let magnitude = delta.unsigned_abs();
        if magnitude < DRIFT_IGNORE_MILLIS {
            // Converged; release any slew.
            self.set_speed(1.0).await;
        } else if magnitude <= DRIFT_HARD_SEEK_MILLIS {
            // Behind the authority → speed up; ahead → slow down.
            let slew = if delta > 0 {
                1.0 + SLEW_RATE
            } else {
                1.0 - SLEW_RATE
            };
            tracing::debug!(delta, slew, "drift: slewing");
            self.set_speed(slew).await;
        } else {
            tracing::info!(delta, target, "drift: hard seek");
            self.set_speed(1.0).await;
            self.seek_programmatic(target).await;
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
                self.eof_reported = false;
                self.note_position(position_millis);
                if self.pending_seek_echoes > 0 {
                    self.pending_seek_echoes -= 1;
                } else {
                    tracing::debug!(position_millis, "user seek (debouncing)");
                    self.pending_user_seek = Some((position_millis, Instant::now()));
                }
            }
            PlayerEvent::Position { position_millis } => {
                self.note_position(position_millis);
            }
            PlayerEvent::DurationKnown { duration_millis } => {
                if let Some((file, _, _)) = &self.current {
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
                if let Some(millis) = self.restore_millis.take() {
                    self.seek_programmatic(millis).await;
                }
                self.apply_desired_pause().await;
            }
            PlayerEvent::PathChanged { path } => {
                // Swallow the echo of our own load (a real file or the
                // placeholder); any other path is one the user loaded
                // directly (drag-and-drop). The session decides whether to
                // adopt it — `self.current.path` is exactly what we last
                // commanded, so a string-equal path is our own.
                let observed = PathBuf::from(path);
                let ours = self
                    .current
                    .as_ref()
                    .is_some_and(|(_, commanded, _)| *commanded == observed);
                if !ours {
                    tracing::info!(
                        path = %observed.display(),
                        "user loaded a file directly into the player"
                    );
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
                if !self.eof_reported
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
        if let Some((position_millis, _)) = self.pending_user_seek.take() {
            tracing::info!(position_millis, "user seek (reporting)");
            let _ = self
                .outputs
                .send(PlayerOutput::UserSeeked { position_millis })
                .await;
        }
    }

    async fn maybe_emit_position(&mut self) {
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
        self.player = None;
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
        match self.factory.spawn().await {
            Ok(player) => {
                tracing::info!("re-attached to mpv");
                self.reattach_backoff = REATTACH_BACKOFF_INITIAL;
                self.player = Some(player);
                if let Some((_, path, title)) = self.current.clone()
                    && let Some(player) = &self.player
                    && let Err(e) = player.load(&path, title.as_deref()).await
                {
                    tracing::warn!("reload after re-attach failed: {e}");
                }
                // Position and pause state are restored on Loaded.
            }
            Err(e) => {
                tracing::debug!(
                    "attached mpv still down ({e}); retrying in {:?}",
                    self.reattach_backoff
                );
                self.reattach_at = Some(Instant::now() + self.reattach_backoff);
                self.reattach_backoff = (self.reattach_backoff * 2).min(REATTACH_BACKOFF_MAX);
            }
        }
    }

    /// Spawn mode: a player we own died. Escalate per design.md — silent
    /// relaunch, a global pause + chat notice on the second death within
    /// [`CRASH_FATAL_WINDOW`], and give up on the third. Returns false
    /// when the actor should exit (relaunch impossible).
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
        self.estimate = None;
        match self.factory.spawn().await {
            Ok(player) => {
                self.player = Some(player);
                if let Some((_, path, title)) = self.current.clone()
                    && let Some(player) = &self.player
                    && let Err(e) = player.load(&path, title.as_deref()).await
                {
                    tracing::warn!("reload after relaunch failed: {e}");
                }
                // Position and pause state are restored on Loaded.
                true
            }
            Err(e) => {
                tracing::error!("relaunch failed: {e}");
                let _ = self.outputs.send(PlayerOutput::FatalCrash).await;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;

    use super::*;
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
            factory: MockFactory::new(vec![]),
            outputs: out_tx,
            clock: fixed_clock(0),
            offset_millis: 0,
            player: None,
            current: None,
            desired_playing: false,
            believed_pause: None,
            speed: 1.0,
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
                position_millis: 90_000
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
        commands
            .send(PlayerCommand::SyncTo {
                position_millis: 20_000,
                timestamp: SharedTimestamp(1_000_000),
                playing: false,
            })
            .await
            .unwrap();
        settle().await;
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
                position_millis: 55_000
            }],
            "the user's seek on the new file was swallowed as a stale echo"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn drift_below_ignore_band_does_nothing() {
        let (commands, _outputs, mut control) = loaded_rig().await;
        commands
            .send(PlayerCommand::SyncTo {
                position_millis: 10_000 + DRIFT_IGNORE_MILLIS - 1,
                timestamp: SharedTimestamp(1_000_000),
                playing: false,
            })
            .await
            .unwrap();
        settle().await;
        assert_eq!(control.drain_commands(), vec![]);
    }

    #[tokio::test(start_paused = true)]
    async fn drift_in_slew_band_slews_and_releases_on_convergence() {
        let (commands, _outputs, mut control) = loaded_rig().await;
        // 1s behind the authority: speed up.
        commands
            .send(PlayerCommand::SyncTo {
                position_millis: 11_000,
                timestamp: SharedTimestamp(1_000_000),
                playing: false,
            })
            .await
            .unwrap();
        settle().await;
        assert_eq!(
            control.drain_commands(),
            vec![MockCommand::SetSpeed(1.0 + SLEW_RATE)]
        );
        // Converged: release the slew.
        commands
            .send(PlayerCommand::SyncTo {
                position_millis: 10_050,
                timestamp: SharedTimestamp(1_000_000),
                playing: false,
            })
            .await
            .unwrap();
        settle().await;
        assert_eq!(control.drain_commands(), vec![MockCommand::SetSpeed(1.0)]);
    }

    #[tokio::test(start_paused = true)]
    async fn drift_ahead_slews_down() {
        let (commands, _outputs, mut control) = loaded_rig().await;
        commands
            .send(PlayerCommand::SyncTo {
                position_millis: 9_000,
                timestamp: SharedTimestamp(1_000_000),
                playing: false,
            })
            .await
            .unwrap();
        settle().await;
        assert_eq!(
            control.drain_commands(),
            vec![MockCommand::SetSpeed(1.0 - SLEW_RATE)]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn drift_beyond_band_hard_seeks_and_suppresses_the_echo() {
        let (commands, mut outputs, mut control) = loaded_rig().await;
        commands
            .send(PlayerCommand::SyncTo {
                position_millis: 20_000,
                timestamp: SharedTimestamp(1_000_000),
                playing: false,
            })
            .await
            .unwrap();
        settle().await;
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
        // ~15_000 now. We're at 10_000 → 5s behind → hard seek.
        commands
            .send(PlayerCommand::SyncTo {
                position_millis: 10_000,
                timestamp: SharedTimestamp(995_000),
                playing: true,
            })
            .await
            .unwrap();
        settle().await;
        assert_eq!(control.drain_commands(), vec![MockCommand::Seek(15_000)]);
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
        // Far past the old 10s SOCKET_WAIT deadline.
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
                speaker: Some("Frieren".into()),
            })
            .unwrap();
        // The in-video position is attached from the actor's estimate;
        // loaded_rig parked it at 10_000 (paused, so it doesn't advance).
        // The speaker rides along unchanged.
        assert_eq!(
            expect_output(&mut outputs).await,
            PlayerOutput::SubtitleLine {
                text: "こんにちは".into(),
                speaker: Some("Frieren".into()),
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
    async fn osd_passes_through_to_the_player() {
        let (commands, _outputs, mut control) = loaded_rig().await;
        commands
            .send(PlayerCommand::ShowOsd("Baughn: hello".into()))
            .await
            .unwrap();
        assert_eq!(
            expect_command(&mut control).await,
            MockCommand::ShowOsd("Baughn: hello".into())
        );
    }
}
