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
    /// `load(path)`.
    Load(PathBuf),
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
}

impl MockPlayer {
    /// A fully manual mock: commands produce no events.
    pub fn pair() -> (MockPlayer, MockControl) {
        Self::build(false)
    }

    /// A mock that acks commands the way mpv would.
    pub fn auto_pair() -> (MockPlayer, MockControl) {
        Self::build(true)
    }

    fn build(auto: bool) -> (MockPlayer, MockControl) {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let player = MockPlayer {
            commands: cmd_tx,
            events: Mutex::new(event_rx),
            auto_ack: auto.then(|| event_tx.clone()),
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
    async fn load(&self, path: &Path) -> Result<(), PlayerError> {
        self.send(MockCommand::Load(path.to_path_buf()))?;
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

/// A [`PlayerFactory`] that hands out pre-built mocks — one per spawn,
/// so crash-relaunch tests can script the second instance differently.
pub struct MockFactory {
    players: VecDeque<MockPlayer>,
    /// How many spawns have happened (relaunch assertions).
    pub spawned: usize,
}

impl MockFactory {
    /// A factory over the given instances, in order.
    pub fn new(players: impl IntoIterator<Item = MockPlayer>) -> Self {
        MockFactory {
            players: players.into_iter().collect(),
            spawned: 0,
        }
    }
}

impl PlayerFactory for MockFactory {
    type Player = MockPlayer;

    async fn spawn(&mut self) -> Result<MockPlayer, PlayerError> {
        self.spawned += 1;
        self.players
            .pop_front()
            .ok_or_else(|| PlayerError::Setup("mock factory exhausted".into()))
    }
}
