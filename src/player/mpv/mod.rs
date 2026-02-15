pub mod ipc;
pub mod protocol;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use serde_json::json;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, info, trace};

use self::ipc::MpvIpc;
use self::protocol::{MpvEvent, OBSERVE_EOF_REACHED, OBSERVE_PAUSE, OBSERVE_TIME_POS};
use super::bridge::PlayerBridge;
use super::error::PlayerError;
use super::events::PlayerEvent;

/// Unique socket path counter for parallel instances.
static SOCKET_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_socket_path() -> PathBuf {
    let pid = std::process::id();
    let counter = SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(format!("/tmp/dessplay-mpv-{pid}-{counter}.sock"))
}

/// Echo suppression attribution — tracks why each event was emitted or suppressed.
#[derive(Debug, Clone)]
pub enum EventAttribution {
    EmittedAsUserAction(PlayerEvent),
    SuppressedByPendingPause,
    SuppressedByPendingSeek,
    SuppressedAsStalePosition,
    EmittedAsPositionUpdate,
}

/// Internal echo suppression state shared between bridge commands and event translator.
#[derive(Debug, Default)]
struct EchoState {
    /// Expected pause value from our programmatic command.
    pending_pause: Option<bool>,
    /// We have a programmatic seek in flight.
    pending_seek: bool,
    /// Saw an unattributed seek event, waiting for the next time-pos to emit UserSeeked.
    awaiting_user_seek_pos: bool,
    /// Suppressing PositionChanged until playback-restart after a programmatic seek.
    suppressing_stale_positions: bool,
    /// Attribution log for test inspection.
    attribution_log: Vec<EventAttribution>,
}

/// mpv player bridge implementation.
///
/// Controls an mpv process via JSON-IPC, translates events with echo suppression.
pub struct MpvPlayer {
    headless: bool,
    socket_path: PathBuf,
    ipc: Option<Arc<MpvIpc>>,
    echo_state: Arc<Mutex<EchoState>>,
    event_task: Option<JoinHandle<()>>,
    child_task: Option<JoinHandle<()>>,
    kill_tx: Option<oneshot::Sender<()>>,
    last_file: Arc<Mutex<Option<PathBuf>>>,
    last_position: Arc<Mutex<Option<f64>>>,
    last_crash: Option<Instant>,
}

impl MpvPlayer {
    /// Create a new MpvPlayer.
    ///
    /// If `headless` is true, mpv runs with `--vo=null --ao=null` (for tests).
    pub fn new(headless: bool) -> Self {
        Self {
            headless,
            socket_path: unique_socket_path(),
            ipc: None,
            echo_state: Arc::new(Mutex::new(EchoState::default())),
            event_task: None,
            child_task: None,
            kill_tx: None,
            last_file: Arc::new(Mutex::new(None)),
            last_position: Arc::new(Mutex::new(None)),
            last_crash: None,
        }
    }

    /// Send a keypress through mpv's input pipeline.
    ///
    /// Does NOT register pending state, so resulting events are treated as user-initiated.
    /// For test use — simulates user input without Wayland/X11.
    pub async fn keypress(&self, key: &str) -> Result<(), PlayerError> {
        let ipc = self.ipc.as_ref().ok_or(PlayerError::NotRunning)?;
        ipc.send_command(vec![json!("keypress"), json!(key)])
            .await?;
        Ok(())
    }

    /// Inspect the echo suppression attribution log (test-only).
    pub fn attribution_log(&self) -> Vec<EventAttribution> {
        self.echo_state.lock().unwrap().attribution_log.clone()
    }

    /// Clear the attribution log.
    pub fn clear_attribution_log(&self) {
        self.echo_state.lock().unwrap().attribution_log.clear();
    }

    /// Get the PID of the mpv process (for test use — crash simulation).
    pub async fn get_pid(&self) -> Result<i32, PlayerError> {
        let ipc = self.ipc.as_ref().ok_or(PlayerError::NotRunning)?;
        let resp = ipc
            .send_command(vec![json!("get_property"), json!("pid")])
            .await?;
        resp.data
            .as_ref()
            .and_then(|v| v.as_i64())
            .map(|v| v as i32)
            .ok_or_else(|| PlayerError::Protocol("pid not a number".to_string()))
    }

    /// Re-spawn after a crash. Reloads last file and seeks to last position.
    ///
    /// Returns error if a second crash happens within 30 seconds.
    pub async fn handle_crash(&mut self) -> Result<mpsc::Receiver<PlayerEvent>, PlayerError> {
        let now = Instant::now();

        if let Some(last) = self.last_crash {
            if now.duration_since(last).as_secs() < 30 {
                return Err(PlayerError::SpawnFailed(
                    "second crash within 30 seconds".to_string(),
                ));
            }
        }

        self.last_crash = Some(now);

        // Save state before re-spawn
        let file = self.last_file.lock().unwrap().clone();
        let position = *self.last_position.lock().unwrap();

        // Re-spawn
        let rx = self.spawn().await?;

        // Restore state
        if let Some(path) = file {
            self.loadfile(&path).await?;
            // Wait a moment for file to load before seeking
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if let Some(pos) = position {
                self.seek(pos).await?;
            }
            self.pause().await?;
        }

        Ok(rx)
    }

    /// Internal: spawn the mpv child process.
    fn spawn_child(&self) -> Result<Child, PlayerError> {
        let mut cmd = Command::new("mpv");
        cmd.arg("--idle=yes")
            .arg("--keep-open=yes")
            .arg(format!(
                "--input-ipc-server={}",
                self.socket_path.display()
            ))
            .arg("--no-terminal")
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if self.headless {
            cmd.arg("--vo=null").arg("--ao=null");
        }

        cmd.spawn()
            .map_err(|e| PlayerError::SpawnFailed(e.to_string()))
    }

    /// Internal: set up property observers on the IPC connection.
    async fn setup_observers(ipc: &MpvIpc) -> Result<(), PlayerError> {
        ipc.send_command(vec![
            json!("observe_property"),
            json!(OBSERVE_TIME_POS),
            json!("time-pos"),
        ])
        .await?;
        ipc.send_command(vec![
            json!("observe_property"),
            json!(OBSERVE_PAUSE),
            json!("pause"),
        ])
        .await?;
        ipc.send_command(vec![
            json!("observe_property"),
            json!(OBSERVE_EOF_REACHED),
            json!("eof-reached"),
        ])
        .await?;
        Ok(())
    }

    /// Internal: spawn the event translator task.
    ///
    /// Reads raw MpvEvents, applies echo suppression, and forwards PlayerEvents.
    fn spawn_event_translator(
        mut mpv_rx: mpsc::Receiver<MpvEvent>,
        player_tx: mpsc::Sender<PlayerEvent>,
        echo_state: Arc<Mutex<EchoState>>,
        last_position: Arc<Mutex<Option<f64>>>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(event) = mpv_rx.recv().await {
                match event {
                    MpvEvent::PropertyChange(prop) => {
                        match (prop.id, prop.name.as_str()) {
                            (OBSERVE_TIME_POS, "time-pos") => {
                                let pos = prop.data.as_ref().and_then(|v| v.as_f64());
                                if let Some(pos) = pos {
                                    // Update last known position
                                    *last_position.lock().unwrap() = Some(pos);

                                    let mut state = echo_state.lock().unwrap();

                                    if state.awaiting_user_seek_pos {
                                        // User-initiated seek completed
                                        state.awaiting_user_seek_pos = false;
                                        let event = PlayerEvent::UserSeeked { position: pos };
                                        state.attribution_log.push(
                                            EventAttribution::EmittedAsUserAction(event.clone()),
                                        );
                                        drop(state);
                                        let _ = player_tx.try_send(event);
                                    } else if state.suppressing_stale_positions {
                                        state.attribution_log.push(
                                            EventAttribution::SuppressedAsStalePosition,
                                        );
                                    } else {
                                        state.attribution_log.push(
                                            EventAttribution::EmittedAsPositionUpdate,
                                        );
                                        drop(state);
                                        let _ =
                                            player_tx.try_send(PlayerEvent::PositionChanged(pos));
                                    }
                                }
                            }
                            (OBSERVE_PAUSE, "pause") => {
                                let paused = prop.data.as_ref().and_then(|v| v.as_bool());
                                if let Some(paused) = paused {
                                    let mut state = echo_state.lock().unwrap();

                                    if state.pending_pause == Some(paused) {
                                        // This matches our programmatic command — suppress
                                        state.pending_pause = None;
                                        state
                                            .attribution_log
                                            .push(EventAttribution::SuppressedByPendingPause);
                                    } else {
                                        // Clear pending (if any) since state changed unexpectedly
                                        state.pending_pause = None;
                                        let event =
                                            PlayerEvent::UserPauseToggled { paused };
                                        state.attribution_log.push(
                                            EventAttribution::EmittedAsUserAction(event.clone()),
                                        );
                                        drop(state);
                                        let _ = player_tx.try_send(event);
                                    }
                                }
                            }
                            (OBSERVE_EOF_REACHED, "eof-reached") => {
                                let eof = prop
                                    .data
                                    .as_ref()
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                if eof {
                                    let _ = player_tx.try_send(PlayerEvent::EndOfFile);
                                }
                            }
                            _ => {
                                trace!("unhandled property change: {}={:?}", prop.name, prop.data);
                            }
                        }
                    }
                    MpvEvent::RawEvent(evt) => match evt.event.as_str() {
                        "seek" => {
                            let mut state = echo_state.lock().unwrap();
                            if state.pending_seek {
                                // Programmatic seek — suppress and start suppressing stale positions
                                state.pending_seek = false;
                                state.suppressing_stale_positions = true;
                                state
                                    .attribution_log
                                    .push(EventAttribution::SuppressedByPendingSeek);
                            } else {
                                // User-initiated seek — wait for next time-pos
                                state.awaiting_user_seek_pos = true;
                                // Also suppress stale positions until playback-restart
                                state.suppressing_stale_positions = true;
                            }
                        }
                        "playback-restart" => {
                            let mut state = echo_state.lock().unwrap();
                            state.suppressing_stale_positions = false;
                        }
                        "shutdown" => {
                            debug!("mpv shutdown event");
                        }
                        _ => {
                            trace!("unhandled mpv event: {}", evt.event);
                        }
                    },
                }
            }
        })
    }

    /// Internal: spawn the child watcher task.
    fn spawn_child_watcher(
        mut child: Child,
        kill_rx: oneshot::Receiver<()>,
        player_tx: mpsc::Sender<PlayerEvent>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            tokio::select! {
                status = child.wait() => {
                    let clean = status.as_ref().map(|s| s.success()).unwrap_or(false);
                    info!("mpv exited: {status:?}");
                    let _ = player_tx.try_send(PlayerEvent::Exited { clean });
                }
                _ = kill_rx => {
                    debug!("killing mpv process");
                    let _ = child.kill().await;
                }
            }
        })
    }

    fn cleanup(&mut self) {
        // Abort tasks
        if let Some(task) = self.event_task.take() {
            task.abort();
        }
        if let Some(task) = self.child_task.take() {
            task.abort();
        }
        // Send kill signal
        if let Some(tx) = self.kill_tx.take() {
            let _ = tx.send(());
        }
        // Drop IPC
        self.ipc = None;
        // Remove socket file
        let _ = std::fs::remove_file(&self.socket_path);
        // Reset echo state
        *self.echo_state.lock().unwrap() = EchoState::default();
    }
}

#[async_trait]
impl PlayerBridge for MpvPlayer {
    async fn spawn(&mut self) -> Result<mpsc::Receiver<PlayerEvent>, PlayerError> {
        // Clean up any previous instance
        self.cleanup();

        // Allocate a fresh socket path
        self.socket_path = unique_socket_path();

        let child = self.spawn_child()?;

        let (ipc, mpv_event_rx) = MpvIpc::connect(&self.socket_path).await?;
        let ipc = Arc::new(ipc);

        Self::setup_observers(&ipc).await?;

        let (player_tx, player_rx) = mpsc::channel(256);

        let event_task = Self::spawn_event_translator(
            mpv_event_rx,
            player_tx.clone(),
            Arc::clone(&self.echo_state),
            Arc::clone(&self.last_position),
        );

        let (kill_tx, kill_rx) = oneshot::channel();
        let child_task = Self::spawn_child_watcher(child, kill_rx, player_tx);

        self.ipc = Some(ipc);
        self.event_task = Some(event_task);
        self.child_task = Some(child_task);
        self.kill_tx = Some(kill_tx);

        Ok(player_rx)
    }

    async fn loadfile(&self, path: &Path) -> Result<(), PlayerError> {
        let ipc = self.ipc.as_ref().ok_or(PlayerError::NotRunning)?;

        let path_str = path
            .to_str()
            .ok_or_else(|| PlayerError::FileNotFound(path.to_path_buf()))?;

        if !path.exists() {
            return Err(PlayerError::FileNotFound(path.to_path_buf()));
        }

        ipc.send_command(vec![json!("loadfile"), json!(path_str)])
            .await?;

        *self.last_file.lock().unwrap() = Some(path.to_path_buf());

        Ok(())
    }

    async fn pause(&self) -> Result<(), PlayerError> {
        let ipc = self.ipc.as_ref().ok_or(PlayerError::NotRunning)?;

        // Register pending BEFORE sending command
        {
            let mut state = self.echo_state.lock().unwrap();
            state.pending_pause = Some(true);
        }

        ipc.send_command(vec![json!("set_property"), json!("pause"), json!(true)])
            .await?;

        Ok(())
    }

    async fn play(&self) -> Result<(), PlayerError> {
        let ipc = self.ipc.as_ref().ok_or(PlayerError::NotRunning)?;

        {
            let mut state = self.echo_state.lock().unwrap();
            state.pending_pause = Some(false);
        }

        ipc.send_command(vec![json!("set_property"), json!("pause"), json!(false)])
            .await?;

        Ok(())
    }

    async fn seek(&self, seconds: f64) -> Result<(), PlayerError> {
        let ipc = self.ipc.as_ref().ok_or(PlayerError::NotRunning)?;

        {
            let mut state = self.echo_state.lock().unwrap();
            state.pending_seek = true;
        }

        ipc.send_command(vec![json!("seek"), json!(seconds), json!("absolute")])
            .await?;

        Ok(())
    }

    async fn show_text(&self, message: &str, duration_ms: u32) -> Result<(), PlayerError> {
        let ipc = self.ipc.as_ref().ok_or(PlayerError::NotRunning)?;
        ipc.send_command(vec![json!("show-text"), json!(message), json!(duration_ms)])
            .await?;
        Ok(())
    }

    async fn get_position(&self) -> Result<Option<f64>, PlayerError> {
        let ipc = self.ipc.as_ref().ok_or(PlayerError::NotRunning)?;

        match ipc
            .send_command(vec![json!("get_property"), json!("time-pos")])
            .await
        {
            Ok(resp) => Ok(resp.data.as_ref().and_then(|v| v.as_f64())),
            Err(PlayerError::CommandError(e)) if e.contains("property unavailable") => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn quit(&self) -> Result<(), PlayerError> {
        let ipc = self.ipc.as_ref().ok_or(PlayerError::NotRunning)?;
        let _ = ipc.send_command_raw(vec![json!("quit")]).await;
        Ok(())
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        self.cleanup();
    }
}
