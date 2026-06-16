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

/// Commands from the main loop.
#[derive(Debug)]
pub enum PlayerCommand {
    /// Load a file (it starts paused; desired state is applied on load).
    Load {
        /// The file's identity, echoed back in [`PlayerOutput::Eof`].
        file: Ed2kHash,
        /// Local path to play.
        path: PathBuf,
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
    /// The player died twice within [`CRASH_FATAL_WINDOW`]. The main
    /// loop should pause globally and say so in chat.
    FatalCrash,
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
    /// Now-loaded file and path (for EOF attribution and relaunch).
    current: Option<(Ed2kHash, PathBuf)>,
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
}

/// Run the player actor until `commands` closes or [`PlayerCommand::Shutdown`].
pub async fn run<F: PlayerFactory>(
    factory: F,
    clock: Clock,
    mut commands: mpsc::Receiver<PlayerCommand>,
    outputs: mpsc::Sender<PlayerOutput>,
) {
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
    };

    match actor.factory.spawn().await {
        Ok(player) => actor.player = Some(player),
        Err(e) => {
            tracing::error!("cannot launch the player: {e}");
            let _ = actor.outputs.send(PlayerOutput::FatalCrash).await;
            return;
        }
    }
    tracing::info!("player launched");

    let mut cadence = tokio::time::interval(POSITION_CADENCE_PLAYING);
    cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let debounce_at = actor.pending_user_seek.map(|(_, at)| at + SEEK_DEBOUNCE);
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

    async fn handle_command(&mut self, cmd: PlayerCommand) {
        match cmd {
            PlayerCommand::Load { file, path } => {
                tracing::info!(path = %path.display(), "loading file");
                self.current = Some((file, path.clone()));
                self.eof_reported = false;
                self.restore_millis = None;
                self.pending_user_seek = None;
                self.estimate = None;
                // The load contract says the file opens paused.
                self.believed_pause = Some(true);
                self.set_speed(1.0).await;
                if let Some(player) = &self.player
                    && let Err(e) = player.load(&path).await
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
                if let Some(now) = self.estimate_now() {
                    self.note_position(now);
                }
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
            if let Some(now) = self.estimate_now() {
                self.note_position(now);
            }
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
                if let Some((file, _)) = &self.current {
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
            PlayerEvent::SubtitleLine(line) => {
                // Capture the in-video position here, where the estimate
                // is freshest; `0` (-> 00:00) is honest before the first
                // position sample.
                let position_millis = self.estimate_now().unwrap_or(0);
                let _ = self
                    .outputs
                    .send(PlayerOutput::SubtitleLine {
                        text: line,
                        position_millis,
                    })
                    .await;
            }
            PlayerEvent::Eof => {
                if !self.eof_reported
                    && let Some((file, _)) = &self.current
                {
                    self.eof_reported = true;
                    tracing::info!("end of file reached");
                    let _ = self.outputs.send(PlayerOutput::Eof { file: *file }).await;
                }
            }
            PlayerEvent::Exited { clean } => {
                return self.handle_player_death(clean).await;
            }
        }
        true
    }

    async fn handle_pause_observation(&mut self, paused: bool) {
        self.believed_pause = Some(paused);
        // Re-anchor extrapolation at the flip.
        if let Some(now) = self.estimate_now() {
            self.note_position(now);
        }
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
                .send(PlayerOutput::PositionTick { position_millis })
                .await;
        }
    }

    /// Relaunch after the player went away. Returns false when the
    /// actor should exit (relaunch impossible).
    async fn handle_player_death(&mut self, clean: bool) -> bool {
        // A clean exit is still unexpected (a deliberate quit goes
        // through Shutdown, which exits before this runs) — the user
        // closed mpv but the session needs a player, so relaunch either
        // way; the fatal window stops a crash loop.
        if clean {
            tracing::info!("player exited; relaunching");
        } else {
            tracing::warn!("player crashed; relaunching");
        }
        self.player = None;
        self.pending_pause_echoes.clear();
        self.pending_seek_echoes = 0;
        self.pending_user_seek = None;
        self.believed_pause = None;

        let now = Instant::now();
        let fatal = self
            .last_death
            .is_some_and(|at| now.duration_since(at) < CRASH_FATAL_WINDOW);
        self.last_death = Some(now);
        if fatal {
            // Twice in quick succession: tell the session (global pause
            // + chat notice). The relaunch below then comes up paused.
            tracing::error!("player died twice within {CRASH_FATAL_WINDOW:?}");
            let _ = self.outputs.send(PlayerOutput::FatalCrash).await;
        }

        self.restore_millis = self.estimate_now();
        self.speed = 1.0;
        self.estimate = None;
        match self.factory.spawn().await {
            Ok(player) => {
                self.player = Some(player);
                if let Some((_, path)) = self.current.clone()
                    && let Some(player) = &self.player
                    && let Err(e) = player.load(&path).await
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
    const BUDGET: Duration = Duration::from_secs(5);

    fn start(
        mocks: Vec<MockPlayer>,
        clock: Clock,
    ) -> (mpsc::Sender<PlayerCommand>, mpsc::Receiver<PlayerOutput>) {
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (out_tx, out_rx) = mpsc::channel(1024);
        tokio::spawn(run(MockFactory::new(mocks), clock, cmd_rx, out_tx));
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
            })
            .await
            .unwrap();
        assert_eq!(
            expect_command(&mut control).await,
            MockCommand::Load("/media/ep1.mkv".into())
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

    #[tokio::test(start_paused = true)]
    async fn load_failure_is_reported_upstream() {
        let (player, _control) = MockPlayer::pair_failing_load();
        let (commands, mut outputs) = start(vec![player], fixed_clock(1_000_000));
        commands
            .send(PlayerCommand::Load {
                file: FILE,
                path: "/gone/ep1.mkv".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            expect_output(&mut outputs).await,
            PlayerOutput::LoadFailed { file: FILE }
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
            if let PlayerOutput::PositionTick { position_millis } = o {
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
            MockCommand::Load("/media/ep1.mkv".into()),
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
    async fn crashes_outside_the_window_are_not_fatal() {
        let (p1, c1) = MockPlayer::pair();
        let (p2, c2) = MockPlayer::pair();
        let (p3, mut c3) = MockPlayer::pair();
        let (commands, mut outputs) = start(vec![p1, p2, p3], fixed_clock(0));
        commands
            .send(PlayerCommand::Load {
                file: FILE,
                path: "/media/ep1.mkv".into(),
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
            .send(PlayerEvent::SubtitleLine("こんにちは".into()))
            .unwrap();
        // The in-video position is attached from the actor's estimate;
        // loaded_rig parked it at 10_000 (paused, so it doesn't advance).
        assert_eq!(
            expect_output(&mut outputs).await,
            PlayerOutput::SubtitleLine {
                text: "こんにちは".into(),
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
            .send(PlayerEvent::SubtitleLine("hi".into()))
            .unwrap();
        assert_eq!(
            expect_output(&mut outputs).await,
            PlayerOutput::SubtitleLine {
                text: "hi".into(),
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
