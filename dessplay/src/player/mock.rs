//! Mock player for deterministic testing.
//!
//! Records all commands sent and allows injecting events via a channel.

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::sync::mpsc;

use super::{Player, PlayerCommand, PlayerEvent};

/// A mock video player that records commands and replays injected events.
pub struct MockPlayer {
    /// All commands sent to the player, in order.
    pub commands: Arc<Mutex<Vec<PlayerCommand>>>,
    /// Channel for injecting events that `recv_event` will return.
    event_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<PlayerEvent>>,
    /// Current position (set by seek/load).
    position: Arc<Mutex<f64>>,
    /// Current duration.
    duration: Arc<Mutex<Option<f64>>>,
    /// Whether the player is "alive".
    alive: Arc<std::sync::atomic::AtomicBool>,
}

/// Handle for injecting events into a [`MockPlayer`].
pub struct MockPlayerHandle {
    pub event_tx: mpsc::UnboundedSender<PlayerEvent>,
    pub commands: Arc<Mutex<Vec<PlayerCommand>>>,
}

impl MockPlayerHandle {
    /// Inject a player event.
    pub fn send_event(&self, event: PlayerEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Get a snapshot of all commands sent to the player.
    #[cfg(test)]
    pub fn commands(&self) -> Vec<PlayerCommand> {
        self.commands.lock().map_or_else(|_| vec![], |c| c.clone())
    }
}

/// Create a new MockPlayer and its control handle.
pub fn create_mock_player() -> (MockPlayer, MockPlayerHandle) {
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let commands: Arc<Mutex<Vec<PlayerCommand>>> = Arc::new(Mutex::new(Vec::new()));

    let player = MockPlayer {
        commands: Arc::clone(&commands),
        event_rx: tokio::sync::Mutex::new(event_rx),
        position: Arc::new(Mutex::new(0.0)),
        duration: Arc::new(Mutex::new(None)),
        alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
    };

    let handle = MockPlayerHandle {
        event_tx,
        commands,
    };

    (player, handle)
}

impl Player for MockPlayer {
    async fn load_file(&self, path: &Path) -> Result<()> {
        if let Ok(mut cmds) = self.commands.lock() {
            cmds.push(PlayerCommand::LoadFile(path.to_path_buf()));
        }
        if let Ok(mut pos) = self.position.lock() {
            *pos = 0.0;
        }
        Ok(())
    }

    async fn pause(&self) -> Result<()> {
        if let Ok(mut cmds) = self.commands.lock() {
            cmds.push(PlayerCommand::Pause);
        }
        Ok(())
    }

    async fn unpause(&self) -> Result<()> {
        if let Ok(mut cmds) = self.commands.lock() {
            cmds.push(PlayerCommand::Unpause);
        }
        Ok(())
    }

    async fn seek(&self, position_secs: f64) -> Result<()> {
        if let Ok(mut cmds) = self.commands.lock() {
            cmds.push(PlayerCommand::Seek(position_secs));
        }
        if let Ok(mut pos) = self.position.lock() {
            *pos = position_secs;
        }
        Ok(())
    }

    async fn get_position(&self) -> Result<f64> {
        Ok(self.position.lock().map_or(0.0, |p| *p))
    }

    async fn get_duration(&self) -> Result<Option<f64>> {
        Ok(self.duration.lock().map_or(None, |d| *d))
    }

    async fn show_osd(&self, text: &str, _duration_ms: u64) -> Result<()> {
        if let Ok(mut cmds) = self.commands.lock() {
            cmds.push(PlayerCommand::ShowOsd(text.to_string()));
        }
        Ok(())
    }

    async fn recv_event(&self) -> Result<PlayerEvent> {
        let mut rx = self.event_rx.lock().await;
        rx.recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("mock player event channel closed"))
    }

    fn is_alive(&self) -> bool {
        self.alive.load(std::sync::atomic::Ordering::Relaxed)
    }

    async fn quit(&self) -> Result<()> {
        self.alive
            .store(false, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[tokio::test]
    async fn records_commands() {
        let (player, handle) = create_mock_player();
        player.load_file(Path::new("/test.mkv")).await.unwrap();
        player.pause().await.unwrap();
        player.seek(42.0).await.unwrap();
        player.unpause().await.unwrap();
        player.show_osd("hello", 3000).await.unwrap();

        let cmds = handle.commands();
        assert_eq!(cmds.len(), 5);
        assert_eq!(
            cmds[0],
            PlayerCommand::LoadFile(PathBuf::from("/test.mkv"))
        );
        assert_eq!(cmds[1], PlayerCommand::Pause);
        assert_eq!(cmds[2], PlayerCommand::Seek(42.0));
        assert_eq!(cmds[3], PlayerCommand::Unpause);
        assert_eq!(cmds[4], PlayerCommand::ShowOsd("hello".to_string()));
    }

    #[tokio::test]
    async fn receives_injected_events() {
        let (player, handle) = create_mock_player();
        handle.send_event(PlayerEvent::Paused);
        handle.send_event(PlayerEvent::Position {
            position_secs: 10.0,
        });

        assert_eq!(player.recv_event().await.unwrap(), PlayerEvent::Paused);
        assert_eq!(
            player.recv_event().await.unwrap(),
            PlayerEvent::Position {
                position_secs: 10.0
            }
        );
    }

    #[tokio::test]
    async fn seek_updates_position() {
        let (player, _handle) = create_mock_player();
        player.seek(55.5).await.unwrap();
        assert!((player.get_position().await.unwrap() - 55.5).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn load_resets_position() {
        let (player, _handle) = create_mock_player();
        player.seek(55.5).await.unwrap();
        player.load_file(Path::new("/new.mkv")).await.unwrap();
        assert!((player.get_position().await.unwrap()).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn quit_marks_not_alive() {
        let (player, _handle) = create_mock_player();
        assert!(player.is_alive());
        player.quit().await.unwrap();
        assert!(!player.is_alive());
    }
}
