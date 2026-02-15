use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::{debug, trace, warn};

use super::protocol::{MpvCommand, MpvEvent, MpvMessage, MpvResponse, parse_message};
use crate::player::error::PlayerError;

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<MpvResponse>>>>;

/// Low-level IPC connection to an mpv process.
///
/// Handles socket I/O, request/response matching, and event routing.
pub struct MpvIpc {
    writer: Mutex<BufWriter<tokio::net::unix::OwnedWriteHalf>>,
    pending: PendingMap,
    next_request_id: AtomicU64,
    reader_task: JoinHandle<()>,
}

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

impl MpvIpc {
    /// Connect to mpv's IPC socket with retry.
    ///
    /// Retries up to 20 times with exponential backoff (50ms initial, 1s max).
    /// Returns the IPC handle and a receiver for mpv events.
    pub async fn connect(
        socket_path: &Path,
    ) -> Result<(Self, mpsc::Receiver<MpvEvent>), PlayerError> {
        let stream = Self::connect_with_retry(socket_path).await?;

        let (read_half, write_half) = stream.into_split();
        let writer = Mutex::new(BufWriter::new(write_half));
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));

        let (event_tx, event_rx) = mpsc::channel(256);

        let reader_task = tokio::spawn(Self::reader_loop(
            read_half,
            Arc::clone(&pending),
            event_tx,
        ));

        Ok((
            Self {
                writer,
                pending,
                next_request_id: AtomicU64::new(1),
                reader_task,
            },
            event_rx,
        ))
    }

    async fn connect_with_retry(socket_path: &Path) -> Result<UnixStream, PlayerError> {
        let mut delay = Duration::from_millis(50);
        let max_delay = Duration::from_secs(1);
        let max_attempts = 20;

        for attempt in 1..=max_attempts {
            match UnixStream::connect(socket_path).await {
                Ok(stream) => {
                    debug!("connected to mpv IPC on attempt {attempt}");
                    return Ok(stream);
                }
                Err(e) if attempt == max_attempts => {
                    return Err(PlayerError::ConnectionFailed(format!(
                        "failed after {max_attempts} attempts: {e}"
                    )));
                }
                Err(_) => {
                    trace!("connection attempt {attempt} failed, retrying in {delay:?}");
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(max_delay);
                }
            }
        }

        unreachable!()
    }

    async fn reader_loop(
        read_half: tokio::net::unix::OwnedReadHalf,
        pending: PendingMap,
        event_tx: mpsc::Sender<MpvEvent>,
    ) {
        let mut reader = BufReader::new(read_half);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => {
                    debug!("mpv IPC socket closed (EOF)");
                    return;
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }

                    trace!("mpv recv: {trimmed}");

                    match parse_message(trimmed) {
                        Ok(MpvMessage::Response(resp)) => {
                            let request_id = resp.request_id;
                            let mut map = pending.lock().await;
                            if let Some(sender) = map.remove(&request_id) {
                                let _ = sender.send(resp);
                            } else {
                                warn!("received response for unknown request_id {request_id}");
                            }
                        }
                        Ok(MpvMessage::PropertyChange(prop)) => {
                            let _ = event_tx.send(MpvEvent::PropertyChange(prop)).await;
                        }
                        Ok(MpvMessage::Event(evt)) => {
                            let _ = event_tx.send(MpvEvent::RawEvent(evt)).await;
                        }
                        Err(e) => {
                            warn!("failed to parse mpv message: {e}");
                        }
                    }
                }
                Err(e) => {
                    debug!("mpv IPC read error: {e}");
                    return;
                }
            }
        }
    }

    /// Send a command to mpv and wait for the response.
    pub async fn send_command(
        &self,
        args: Vec<serde_json::Value>,
    ) -> Result<MpvResponse, PlayerError> {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);

        let cmd = MpvCommand {
            command: args,
            request_id,
        };

        let json =
            serde_json::to_string(&cmd).map_err(|e| PlayerError::SendFailed(e.to_string()))?;

        trace!("mpv send: {json}");

        let (tx, rx) = oneshot::channel();

        {
            let mut map = self.pending.lock().await;
            map.insert(request_id, tx);
        }

        {
            let mut writer = self.writer.lock().await;
            writer
                .write_all(json.as_bytes())
                .await
                .map_err(|e| PlayerError::SendFailed(e.to_string()))?;
            writer
                .write_all(b"\n")
                .await
                .map_err(|e| PlayerError::SendFailed(e.to_string()))?;
            writer
                .flush()
                .await
                .map_err(|e| PlayerError::SendFailed(e.to_string()))?;
        }

        match tokio::time::timeout(COMMAND_TIMEOUT, rx).await {
            Ok(Ok(resp)) => {
                if resp.error != "success" {
                    Err(PlayerError::CommandError(resp.error))
                } else {
                    Ok(resp)
                }
            }
            Ok(Err(_)) => Err(PlayerError::ReceiveFailed(
                "response channel closed".to_string(),
            )),
            Err(_) => {
                let mut map = self.pending.lock().await;
                map.remove(&request_id);
                Err(PlayerError::Timeout(COMMAND_TIMEOUT))
            }
        }
    }

    /// Send a command without checking the response for errors.
    /// Used for commands like `quit` where mpv may close before responding.
    pub async fn send_command_raw(
        &self,
        args: Vec<serde_json::Value>,
    ) -> Result<Option<MpvResponse>, PlayerError> {
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);

        let cmd = MpvCommand {
            command: args,
            request_id,
        };

        let json =
            serde_json::to_string(&cmd).map_err(|e| PlayerError::SendFailed(e.to_string()))?;

        trace!("mpv send (raw): {json}");

        let (tx, rx) = oneshot::channel();

        {
            let mut map = self.pending.lock().await;
            map.insert(request_id, tx);
        }

        {
            let mut writer = self.writer.lock().await;
            if let Err(e) = writer.write_all(json.as_bytes()).await {
                let mut map = self.pending.lock().await;
                map.remove(&request_id);
                return Err(PlayerError::SendFailed(e.to_string()));
            }
            let _ = writer.write_all(b"\n").await;
            let _ = writer.flush().await;
        }

        match tokio::time::timeout(Duration::from_secs(2), rx).await {
            Ok(Ok(resp)) => Ok(Some(resp)),
            _ => Ok(None),
        }
    }
}

impl Drop for MpvIpc {
    fn drop(&mut self) {
        self.reader_task.abort();
    }
}
