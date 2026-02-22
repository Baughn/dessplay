//! mpv player bridge via JSON IPC over Unix socket.
//!
//! Launches mpv with `--idle=yes --input-ipc-server=<socket>`, communicates
//! via JSON commands, and maps property changes to [`PlayerEvent`]s.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

use super::{Player, PlayerEvent};

/// mpv player instance communicating via JSON IPC.
pub struct MpvPlayer {
    /// Write half of the IPC socket (behind a tokio Mutex for Send).
    writer: tokio::sync::Mutex<tokio::io::WriteHalf<UnixStream>>,
    /// Channel receiving parsed player events from the reader task.
    event_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<PlayerEvent>>,
    /// Monotonically increasing request ID for JSON IPC commands.
    next_request_id: AtomicU64,
    /// Whether the child process is alive.
    alive: Arc<AtomicBool>,
    /// Path to the IPC socket file (for cleanup).
    socket_path: PathBuf,
    /// Child process handle.
    child: tokio::sync::Mutex<tokio::process::Child>,
}

impl MpvPlayer {
    /// Launch mpv and connect to its IPC socket.
    ///
    /// The player starts in idle mode (no file loaded). Use [`Player::load_file`]
    /// to load a video.
    pub async fn launch() -> Result<Self> {
        let pid = std::process::id();
        let socket_path = PathBuf::from(format!("/tmp/dessplay-mpv-{pid}.sock"));

        // Remove stale socket if it exists
        let _ = tokio::fs::remove_file(&socket_path).await;

        let child = tokio::process::Command::new("mpv")
            .arg("--idle=yes")
            .arg(format!(
                "--input-ipc-server={}",
                socket_path.display()
            ))
            .arg("--no-terminal")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("failed to launch mpv")?;

        // Wait for the socket to appear (100ms intervals, 3s timeout)
        let mut attempts = 0;
        loop {
            if socket_path.exists() {
                break;
            }
            attempts += 1;
            if attempts > 30 {
                anyhow::bail!(
                    "mpv IPC socket did not appear at {} within 3 seconds",
                    socket_path.display()
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        let stream = UnixStream::connect(&socket_path)
            .await
            .context("failed to connect to mpv IPC socket")?;

        let (read_half, write_half) = tokio::io::split(stream);

        let alive = Arc::new(AtomicBool::new(true));
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        // Spawn reader task
        let alive_clone = Arc::clone(&alive);
        tokio::spawn(async move {
            reader_task(read_half, event_tx, alive_clone).await;
        });

        let player = Self {
            writer: tokio::sync::Mutex::new(write_half),
            event_rx: tokio::sync::Mutex::new(event_rx),
            next_request_id: AtomicU64::new(1),
            alive,
            socket_path,
            child: tokio::sync::Mutex::new(child),
        };

        // Subscribe to property changes
        player.observe_property("pause", 1).await?;
        player.observe_property("time-pos", 2).await?;
        player.observe_property("duration", 3).await?;

        Ok(player)
    }

    /// Send a raw JSON command to mpv.
    async fn send_command(&self, args: &[serde_json::Value]) -> Result<()> {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let cmd = serde_json::json!({
            "command": args,
            "request_id": request_id,
        });
        let mut line = serde_json::to_string(&cmd).context("failed to serialize mpv command")?;
        line.push('\n');

        let mut writer = self.writer.lock().await;
        writer
            .write_all(line.as_bytes())
            .await
            .context("failed to write to mpv IPC")?;
        writer.flush().await.context("failed to flush mpv IPC")?;
        Ok(())
    }

    /// Subscribe to a property via `observe_property`.
    async fn observe_property(&self, property: &str, id: u64) -> Result<()> {
        self.send_command(&[
            serde_json::json!("observe_property"),
            serde_json::json!(id),
            serde_json::json!(property),
        ])
        .await
    }
}

impl Player for MpvPlayer {
    async fn load_file(&self, path: &Path) -> Result<()> {
        let path_str = path.to_string_lossy();
        self.send_command(&[
            serde_json::json!("loadfile"),
            serde_json::json!(path_str),
        ])
        .await
    }

    async fn pause(&self) -> Result<()> {
        self.send_command(&[
            serde_json::json!("set_property"),
            serde_json::json!("pause"),
            serde_json::json!(true),
        ])
        .await
    }

    async fn unpause(&self) -> Result<()> {
        self.send_command(&[
            serde_json::json!("set_property"),
            serde_json::json!("pause"),
            serde_json::json!(false),
        ])
        .await
    }

    async fn seek(&self, position_secs: f64) -> Result<()> {
        self.send_command(&[
            serde_json::json!("seek"),
            serde_json::json!(position_secs),
            serde_json::json!("absolute"),
        ])
        .await
    }

    async fn get_position(&self) -> Result<f64> {
        // We rely on property observation for position; this is a fallback.
        // Send get_property — response will come through the reader task,
        // but since we observe time-pos, we can just return 0.0 here.
        // The real position comes via PlayerEvent::Position.
        Ok(0.0)
    }

    async fn get_duration(&self) -> Result<Option<f64>> {
        Ok(None)
    }

    async fn show_osd(&self, text: &str, duration_ms: u64) -> Result<()> {
        self.send_command(&[
            serde_json::json!("show-text"),
            serde_json::json!(text),
            serde_json::json!(duration_ms),
        ])
        .await
    }

    async fn recv_event(&self) -> Result<PlayerEvent> {
        let mut rx = self.event_rx.lock().await;
        rx.recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("mpv event channel closed"))
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    async fn quit(&self) -> Result<()> {
        self.send_command(&[serde_json::json!("quit")])
            .await
            .ok();
        self.alive.store(false, Ordering::Relaxed);

        // Wait for the child to exit
        let mut child = self.child.lock().await;
        let _ = child.kill().await;

        // Clean up socket
        let _ = tokio::fs::remove_file(&self.socket_path).await;
        Ok(())
    }
}

impl Drop for MpvPlayer {
    fn drop(&mut self) {
        // Best-effort cleanup of the socket file
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Background task that reads JSON lines from mpv and sends parsed events.
async fn reader_task(
    read_half: tokio::io::ReadHalf<UnixStream>,
    event_tx: mpsc::UnboundedSender<PlayerEvent>,
    alive: Arc<AtomicBool>,
) {
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) => {
                // EOF — mpv process exited
                alive.store(false, Ordering::Relaxed);
                let _ = event_tx.send(PlayerEvent::Crashed);
                break;
            }
            Ok(_) => {
                if let Some(event) = parse_mpv_event(&line)
                    && event_tx.send(event).is_err()
                {
                    break;
                }
            }
            Err(e) => {
                tracing::debug!("mpv IPC read error: {e}");
                alive.store(false, Ordering::Relaxed);
                let _ = event_tx.send(PlayerEvent::Crashed);
                break;
            }
        }
    }
}

/// Parse a JSON line from mpv into a PlayerEvent, if applicable.
fn parse_mpv_event(line: &str) -> Option<PlayerEvent> {
    let json: serde_json::Value = serde_json::from_str(line.trim()).ok()?;

    // Property change events
    if json.get("event").and_then(|v| v.as_str()) == Some("property-change") {
        let name = json.get("name")?.as_str()?;
        match name {
            "pause" => {
                let paused = json.get("data")?.as_bool()?;
                if paused {
                    return Some(PlayerEvent::Paused);
                } else {
                    return Some(PlayerEvent::Unpaused);
                }
            }
            "time-pos" => {
                let pos = json.get("data")?.as_f64()?;
                return Some(PlayerEvent::Position {
                    position_secs: pos,
                });
            }
            "duration" => {
                let dur = json.get("data")?.as_f64()?;
                return Some(PlayerEvent::Duration {
                    duration_secs: dur,
                });
            }
            _ => {}
        }
    }

    // Seek event
    if json.get("event").and_then(|v| v.as_str()) == Some("seek") {
        // mpv fires "seek" when a seek starts. We'll pick up the position from
        // the subsequent time-pos property change. But we can also check
        // "playback-restart" for the final position. For now, we rely on
        // time-pos property observation and emit Seeked on significant jumps
        // in the runner.
        return None;
    }

    // End of file
    if json.get("event").and_then(|v| v.as_str()) == Some("end-file") {
        let reason = json
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        if reason == "eof" {
            return Some(PlayerEvent::Eof);
        } else if reason == "error" || reason == "quit" {
            // Process exit handled by reader loop EOF
            return None;
        }
    }

    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_pause_event() {
        let json = r#"{"event":"property-change","id":1,"name":"pause","data":true}"#;
        assert_eq!(parse_mpv_event(json), Some(PlayerEvent::Paused));
    }

    #[test]
    fn parse_unpause_event() {
        let json = r#"{"event":"property-change","id":1,"name":"pause","data":false}"#;
        assert_eq!(parse_mpv_event(json), Some(PlayerEvent::Unpaused));
    }

    #[test]
    fn parse_position_event() {
        let json = r#"{"event":"property-change","id":2,"name":"time-pos","data":42.5}"#;
        assert_eq!(
            parse_mpv_event(json),
            Some(PlayerEvent::Position {
                position_secs: 42.5
            })
        );
    }

    #[test]
    fn parse_duration_event() {
        let json = r#"{"event":"property-change","id":3,"name":"duration","data":1440.0}"#;
        assert_eq!(
            parse_mpv_event(json),
            Some(PlayerEvent::Duration {
                duration_secs: 1440.0
            })
        );
    }

    #[test]
    fn parse_eof_event() {
        let json = r#"{"event":"end-file","reason":"eof"}"#;
        assert_eq!(parse_mpv_event(json), Some(PlayerEvent::Eof));
    }

    #[test]
    fn parse_unknown_event() {
        let json = r#"{"event":"idle"}"#;
        assert_eq!(parse_mpv_event(json), None);
    }

    #[test]
    fn parse_command_response() {
        let json = r#"{"request_id":1,"error":"success"}"#;
        assert_eq!(parse_mpv_event(json), None);
    }

    #[cfg(feature = "mpv-tests")]
    mod integration {
        use super::*;

        #[tokio::test]
        async fn launch_and_quit() {
            let player = MpvPlayer::launch().await.unwrap();
            assert!(player.is_alive());
            player.quit().await.unwrap();
            assert!(!player.is_alive());
        }
    }
}
