use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("failed to spawn player process: {0}")]
    SpawnFailed(String),

    #[error("failed to connect to player IPC: {0}")]
    ConnectionFailed(String),

    #[error("failed to send command: {0}")]
    SendFailed(String),

    #[error("failed to receive response: {0}")]
    ReceiveFailed(String),

    #[error("command error: {0}")]
    CommandError(String),

    #[error("command timed out after {0:?}")]
    Timeout(Duration),

    #[error("player process exited with code: {0:?}")]
    ProcessExited(Option<i32>),

    #[error("player is not running")]
    NotRunning,

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("file not found: {0}")]
    FileNotFound(PathBuf),
}
