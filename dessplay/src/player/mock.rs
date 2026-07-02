//! In-process [`Player`] double for deterministic tests.
//!
//! A mock is created as a pair: the [`MockPlayer`] handed to the actor,
//! and the [`MockControl`] kept by the test, which observes every
//! command the actor sent and injects whatever [`PlayerEvent`]s it
//! likes ("the user pressed space", "mpv crashed"...).
//!
//! Two flavors:
//! - [`MockPlayer::pair`] is fully manual — commands produce no events,
//!   the test scripts everything. Echo-suppression tests need this
//!   precision.
//! - [`MockPlayer::auto_pair`] acks commands the way mpv would
//!   (`set_pause` fires `PauseChanged`, `seek` fires `Seeked`, `load`
//!   fires `Loaded` + `DurationKnown`), so scenario tests don't have to
//!   hand-echo every command.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use tokio::sync::{Mutex, mpsc};

use super::{Player, PlayerError, PlayerEvent, PlayerFactory};

/// Duration auto-acked for every load: 24 minutes, a typical episode.
pub const AUTO_DURATION_MILLIS: u64 = 24 * 60 * 1000;

/// A command the actor sent to the mock, verbatim.
#[derive(Clone, Debug, PartialEq)]
pub enum MockCommand {
    /// `load(path, title)`.
    Load(PathBuf, Option<String>),
    /// `set_pause(paused)`.
    SetPause(bool),
    /// `seek(position_millis)`.
    Seek(u64),
    /// `set_speed(speed)`.
    SetSpeed(f64),
    /// `show_osd(text)`.
    ShowOsd(String),
    /// `shutdown()`.
    Shutdown,
}

/// The test's side of a mock player.
pub struct MockControl {
    /// Every command the actor sent, in order.
    pub commands: mpsc::UnboundedReceiver<MockCommand>,
    /// Inject observations for the actor's `recv()`.
    pub events: mpsc::UnboundedSender<PlayerEvent>,
}

impl MockControl {
    /// The next command, if one has already been sent (non-blocking).
    pub fn try_command(&mut self) -> Option<MockCommand> {
        self.commands.try_recv().ok()
    }

    /// Drain every command sent so far (non-blocking).
    pub fn drain_commands(&mut self) -> Vec<MockCommand> {
        let mut out = Vec::new();
        while let Ok(cmd) = self.commands.try_recv() {
            out.push(cmd);
        }
        out
    }
}

/// The actor's side of a mock player.
pub struct MockPlayer {
    commands: mpsc::UnboundedSender<MockCommand>,
    events: Mutex<mpsc::UnboundedReceiver<PlayerEvent>>,
    /// Loopback for auto-acks; `None` in manual mode.
    auto_ack: Option<mpsc::UnboundedSender<PlayerEvent>>,
    /// When true, `load` returns an error (simulates a gone/unreadable
    /// file) instead of succeeding.
    load_fails: bool,
}

impl MockPlayer {
    /// A fully manual mock: commands produce no events.
    pub fn pair() -> (MockPlayer, MockControl) {
        Self::build(false, false)
    }

    /// A mock that acks commands the way mpv would.
    pub fn auto_pair() -> (MockPlayer, MockControl) {
        Self::build(true, false)
    }

    /// A manual mock whose `load` always fails (the file is gone).
    pub fn pair_failing_load() -> (MockPlayer, MockControl) {
        Self::build(false, true)
    }

    fn build(auto: bool, load_fails: bool) -> (MockPlayer, MockControl) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let player = MockPlayer {
            commands: cmd_tx,
            events: Mutex::new(event_rx),
            auto_ack: auto.then(|| event_tx.clone()),
            load_fails,
        };
        let control = MockControl {
            commands: cmd_rx,
            events: event_tx,
        };
        (player, control)
    }

    fn send(&self, cmd: MockCommand) -> Result<(), PlayerError> {
        self.commands
            .send(cmd)
            .map_err(|_| PlayerError::Gone("mock control dropped".into()))
    }

    fn ack(&self, event: PlayerEvent) {
        if let Some(tx) = &self.auto_ack {
            let _ = tx.send(event);
        }
    }
}

impl Player for MockPlayer {
    async fn load(&self, path: &Path, title: Option<&str>) -> Result<(), PlayerError> {
        self.send(MockCommand::Load(
            path.to_path_buf(),
            title.map(str::to_owned),
        ))?;
        if self.load_fails {
            return Err(PlayerError::Gone("mock load failure".into()));
        }
        self.ack(PlayerEvent::Loaded);
        self.ack(PlayerEvent::DurationKnown {
            duration_millis: AUTO_DURATION_MILLIS,
        });
        Ok(())
    }

    async fn set_pause(&self, paused: bool) -> Result<(), PlayerError> {
        self.send(MockCommand::SetPause(paused))?;
        self.ack(PlayerEvent::PauseChanged(paused));
        Ok(())
    }

    async fn seek(&self, position_millis: u64) -> Result<(), PlayerError> {
        self.send(MockCommand::Seek(position_millis))?;
        self.ack(PlayerEvent::Seeked { position_millis });
        Ok(())
    }

    async fn set_speed(&self, speed: f64) -> Result<(), PlayerError> {
        self.send(MockCommand::SetSpeed(speed))
    }

    async fn show_osd(&self, text: &str) -> Result<(), PlayerError> {
        self.send(MockCommand::ShowOsd(text.to_string()))
    }

    async fn recv(&self) -> Result<PlayerEvent, PlayerError> {
        self.events
            .lock()
            .await
            .recv()
            .await
            .ok_or_else(|| PlayerError::Gone("mock control dropped".into()))
    }

    async fn shutdown(&self) {
        let _ = self.send(MockCommand::Shutdown);
        self.ack(PlayerEvent::Exited { clean: true });
    }
}

/// One scripted result of a [`MockFactory::spawn`] call.
enum SpawnOutcome {
    /// Hand out this player instance.
    Up(MockPlayer),
    /// Report the player as unavailable: `spawn` returns an error. In
    /// attach mode this models the user's mpv socket still being down,
    /// so the actor's re-attach retry can be exercised deterministically.
    Down,
    /// `spawn` never resolves — models an attach spawn stuck in
    /// `wait_for_socket` against a socket that never comes up, so the
    /// re-attach probe's timeout bound can be exercised.
    Hang,
}

/// A [`PlayerFactory`] that hands out pre-built mocks — one per spawn,
/// so crash-relaunch (and attach re-attach) tests can script each
/// instance, including simulated "still down" attempts.
pub struct MockFactory {
    outcomes: VecDeque<SpawnOutcome>,
    /// Whether this factory reports attach mode (see [`PlayerFactory::is_attach`]).
    attach: bool,
    /// How many spawns have happened (relaunch assertions).
    pub spawned: usize,
}

impl MockFactory {
    /// A spawn-mode factory over the given instances, in order.
    pub fn new(players: impl IntoIterator<Item = MockPlayer>) -> Self {
        MockFactory {
            outcomes: players.into_iter().map(SpawnOutcome::Up).collect(),
            attach: false,
            spawned: 0,
        }
    }

    /// An attach-mode factory (`is_attach() == true`) over the given
    /// instances, in order. Interleave simulated-down attempts with
    /// [`then_down`](Self::then_down) / [`then_up`](Self::then_up).
    pub fn attach(players: impl IntoIterator<Item = MockPlayer>) -> Self {
        MockFactory {
            outcomes: players.into_iter().map(SpawnOutcome::Up).collect(),
            attach: true,
            spawned: 0,
        }
    }

    /// Queue a "player unavailable" result: the next `spawn` after the
    /// already-queued ones returns an error (attach mode: mpv's socket is
    /// not up yet). Builder-style.
    pub fn then_down(mut self) -> Self {
        self.outcomes.push_back(SpawnOutcome::Down);
        self
    }

    /// Queue another player to hand out after the already-queued outcomes.
    /// Builder-style; pairs with [`then_down`](Self::then_down) to script
    /// "down for a while, then mpv returns".
    pub fn then_up(mut self, player: MockPlayer) -> Self {
        self.outcomes.push_back(SpawnOutcome::Up(player));
        self
    }

    /// Queue a spawn that never resolves (models a hung `wait_for_socket`).
    /// Builder-style; exercises the re-attach probe's timeout bound.
    pub fn then_hang(mut self) -> Self {
        self.outcomes.push_back(SpawnOutcome::Hang);
        self
    }
}

impl PlayerFactory for MockFactory {
    type Player = MockPlayer;

    async fn spawn(&mut self) -> Result<MockPlayer, PlayerError> {
        self.spawned += 1;
        match self.outcomes.pop_front() {
            Some(SpawnOutcome::Up(player)) => Ok(player),
            Some(SpawnOutcome::Down) => Err(PlayerError::Setup("mock mpv still down".into())),
            Some(SpawnOutcome::Hang) => {
                // Never resolves: the caller's timeout must fire.
                std::future::pending::<()>().await;
                unreachable!("pending() never resolves")
            }
            None => Err(PlayerError::Setup("mock factory exhausted".into())),
        }
    }

    fn is_attach(&self) -> bool {
        self.attach
    }
}
