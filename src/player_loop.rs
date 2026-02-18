use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};

use crate::network::PeerId;
use crate::network::sync::LocalEvent;
use crate::player::bridge::PlayerBridge;
use crate::player::events::PlayerEvent;
use crate::player::mpv::MpvPlayer;
use crate::state::types::UserState;
use crate::state::SharedState;
use crate::storage::Database;

/// Information about the currently playing file (for watch history tracking).
pub struct CurrentFileInfo {
    pub filename: String,
    pub directory: String,
}

/// Reads PlayerEvents from mpv and translates them to LocalEvents for the sync engine.
///
/// Also updates the player position watch channel so the control loop can
/// compare the player's actual position against the shared state without IPC.
///
/// When a `Database` and `current_file_info_rx` are provided, also tracks
/// watch history (position progress and 90% watched threshold).
pub async fn player_event_loop(
    mut player_rx: mpsc::Receiver<PlayerEvent>,
    local_event_tx: mpsc::UnboundedSender<LocalEvent>,
    player_pos_tx: watch::Sender<f64>,
    db: Option<Arc<Database>>,
    current_file_info_rx: Option<watch::Receiver<Option<CurrentFileInfo>>>,
) {
    let mut duration: Option<f64> = None;
    let mut last_db_write = tokio::time::Instant::now();

    while let Some(event) = player_rx.recv().await {
        match event {
            PlayerEvent::PositionChanged(pos) => {
                let _ = player_pos_tx.send(pos);
                let _ = local_event_tx.send(LocalEvent::PositionUpdated { position: pos });

                // Throttle watch history DB writes to every 10 seconds
                if let (Some(db), Some(info_rx)) = (&db, &current_file_info_rx) {
                    let now = tokio::time::Instant::now();
                    if now.duration_since(last_db_write) > std::time::Duration::from_secs(10) {
                        let dur = duration.unwrap_or(0.0);
                        if let Some(info) = &*info_rx.borrow()
                            && let Err(e) = db.record_watch_progress(
                                &info.filename,
                                &info.directory,
                                pos,
                                dur,
                            )
                        {
                            warn!("failed to record watch progress: {e}");
                        }
                        last_db_write = now;
                    }
                }
            }
            PlayerEvent::UserSeeked { position } => {
                let _ = player_pos_tx.send(position);
                let _ = local_event_tx.send(LocalEvent::PositionUpdated { position });
            }
            PlayerEvent::UserPauseToggled { paused: true } => {
                let _ = local_event_tx.send(LocalEvent::UserStateChanged(UserState::Paused));
            }
            PlayerEvent::UserPauseToggled { paused: false } => {
                let _ = local_event_tx.send(LocalEvent::UserStateChanged(UserState::Ready));
            }
            PlayerEvent::DurationChanged(dur) => {
                duration = Some(dur);
            }
            PlayerEvent::EndOfFile => {
                // Mark as watched (EOF implies we reached the end)
                if let (Some(db), Some(info_rx)) = (&db, &current_file_info_rx)
                    && let Some(info) = &*info_rx.borrow()
                    && let Err(e) = db.mark_watched(&info.filename, &info.directory)
                {
                    warn!("failed to mark file as watched: {e}");
                }
                // TODO: advance playlist
            }
            PlayerEvent::Exited { clean } => {
                info!("player exited (clean={clean}), stopping event loop");
                // TODO: relaunch / quit
                break;
            }
        }
    }
}

/// Watches SharedState changes and commands the player accordingly.
///
/// Detects file changes, play/pause transitions, and remote seeks.
/// Echo suppression in MpvPlayer prevents feedback loops.
///
/// The `resolved_path_rx` channel receives the local file path for the
/// current playlist item (resolved by the TUI from media roots).
pub async fn player_control_loop(
    shared_state: Arc<SharedState>,
    player: Arc<MpvPlayer>,
    _local_peer: PeerId,
    mut version_rx: watch::Receiver<u64>,
    player_pos_rx: watch::Receiver<f64>,
    mut resolved_path_rx: watch::Receiver<Option<PathBuf>>,
) {
    let mut last_file = None;
    let mut last_playing = false;
    let mut last_seek_time = tokio::time::Instant::now();
    let mut current_path: Option<PathBuf> = None;

    loop {
        tokio::select! {
            result = version_rx.changed() => {
                if result.is_err() {
                    break; // sender dropped
                }
            }
            result = resolved_path_rx.changed() => {
                if result.is_err() {
                    break; // sender dropped
                }
                current_path = resolved_path_rx.borrow().clone();
            }
        }

        let view = shared_state.view();

        // File change — load the resolved local file
        if view.current_file != last_file {
            last_file.clone_from(&view.current_file);
            if let Some(path) = &current_path {
                if let Err(e) = player.loadfile(path).await {
                    error!("failed to load file: {e}");
                }
                // mpv may reset pause state on file load — re-assert desired state
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let result = if view.is_playing {
                    player.play().await
                } else {
                    player.pause().await
                };
                if let Err(e) = result {
                    error!("failed to set play state after file load: {e}");
                }
                last_playing = view.is_playing;
            }
            continue;
        }

        // Play/pause transition
        if view.is_playing != last_playing {
            last_playing = view.is_playing;
            let result = if view.is_playing {
                player.play().await
            } else {
                player.pause().await
            };
            if let Err(e) = result {
                error!("failed to set play state: {e}");
            }
        }

        // Remote seek — only if position differs by more than 3 seconds
        let player_pos = *player_pos_rx.borrow();
        if (view.position - player_pos).abs() > 3.0 {
            let now = tokio::time::Instant::now();
            if now.duration_since(last_seek_time) > std::time::Duration::from_secs(1) {
                last_seek_time = now;
                if let Err(e) = player.seek(view.position).await {
                    error!("failed to seek: {e}");
                }
            }
        }
    }
}
