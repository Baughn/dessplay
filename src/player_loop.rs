use std::path::Path;
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use tracing::{error, info};

use crate::network::PeerId;
use crate::network::sync::LocalEvent;
use crate::player::bridge::PlayerBridge;
use crate::player::events::PlayerEvent;
use crate::player::mpv::MpvPlayer;
use crate::state::types::UserState;
use crate::state::SharedState;

/// Reads PlayerEvents from mpv and translates them to LocalEvents for the sync engine.
///
/// Also updates the player position watch channel so the control loop can
/// compare the player's actual position against the shared state without IPC.
pub async fn player_event_loop(
    mut player_rx: mpsc::Receiver<PlayerEvent>,
    local_event_tx: mpsc::UnboundedSender<LocalEvent>,
    player_pos_tx: watch::Sender<f64>,
) {
    while let Some(event) = player_rx.recv().await {
        match event {
            PlayerEvent::PositionChanged(pos) => {
                let _ = player_pos_tx.send(pos);
                let _ = local_event_tx.send(LocalEvent::PositionUpdated { position: pos });
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
            PlayerEvent::EndOfFile => {
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
pub async fn player_control_loop(
    shared_state: Arc<SharedState>,
    player: Arc<MpvPlayer>,
    _local_peer: PeerId,
    mut version_rx: watch::Receiver<u64>,
    player_pos_rx: watch::Receiver<f64>,
) {
    let mut last_file = None;
    let mut last_playing = false;
    let mut last_seek_time = tokio::time::Instant::now();

    loop {
        if version_rx.changed().await.is_err() {
            break; // sender dropped
        }

        let view = shared_state.view();

        // File change — load the stub test video regardless of ItemId
        if view.current_file != last_file {
            last_file.clone_from(&view.current_file);
            if last_file.is_some() {
                let path = Path::new("testdata/video.mkv");
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
