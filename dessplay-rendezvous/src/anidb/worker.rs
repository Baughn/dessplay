//! Background worker that polls the AniDB queue and performs lookups.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use dessplay_core::protocol::{CrdtOp, LwwValue, PeerControl, RvControl};
use dessplay_core::sync_engine::{SyncAction, SyncEngine};
use dessplay_core::types::PeerId;

use crate::anidb::client::{AniDbSession, LookupResult};
use crate::server::ConnectedClient;
use crate::storage::ServerStorage;

/// Run the AniDB lookup worker loop.
///
/// Polls `storage.get_next_pending()`, calls AniDB, writes results as CRDT ops,
/// and broadcasts to connected clients. Sleeps 10s when the queue is empty.
pub(crate) async fn run(
    sync_engine: Arc<tokio::sync::Mutex<SyncEngine>>,
    storage: Arc<std::sync::Mutex<ServerStorage>>,
    clients: Arc<tokio::sync::RwLock<HashMap<PeerId, ConnectedClient>>>,
    anidb_user: String,
    anidb_password: String,
) {
    let mut session = match AniDbSession::new(anidb_user, anidb_password).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to create AniDB session: {e:#}");
            return;
        }
    };

    tracing::info!("AniDB worker started");

    loop {
        if session.is_banned() {
            tracing::error!("AniDB worker stopping: server banned us");
            return;
        }

        // Get next pending file from queue
        let pending = {
            let Ok(s) = storage.lock() else {
                tracing::warn!("AniDB worker: storage lock poisoned");
                return;
            };
            s.get_next_pending(now_millis())
        };

        let (file_id, file_size) = match pending {
            Ok(Some(entry)) => entry,
            Ok(None) => {
                // Queue empty — sleep and retry
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
            Err(e) => {
                tracing::warn!("AniDB worker: failed to get next pending: {e}");
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };

        tracing::info!(%file_id, file_size, "AniDB lookup");

        // Check if user has overridden AniDB data (UserOverAniDb) — skip if so
        {
            let engine = sync_engine.lock().await;
            if let Some(existing) = engine.state().anidb.read(&file_id) {
                if let Some(meta) = existing {
                    if meta.source == dessplay_core::types::MetadataSource::UserOverAniDb {
                        tracing::info!(%file_id, "Skipping AniDB lookup: user override");
                        if let Ok(s) = storage.lock() {
                            let _ = s.record_success(&file_id, now_millis());
                        }
                        continue;
                    }
                }
            }
        }

        match session.lookup_file(&file_id, file_size).await {
            Ok(LookupResult::Found(metadata)) => {
                tracing::info!(
                    %file_id,
                    anime = %metadata.anime_name,
                    ep = %metadata.episode_number,
                    "AniDB: file found"
                );

                // Create CRDT op (server-authoritative write)
                let now = now_millis();
                let op = CrdtOp::LwwWrite {
                    timestamp: now,
                    value: LwwValue::AniDb(file_id, Some(metadata)),
                };

                // Apply and broadcast
                let actions = sync_engine.lock().await.apply_local_op(op);
                dispatch_anidb_actions(actions, &clients, &storage, &sync_engine).await;

                // Record success
                if let Ok(s) = storage.lock()
                    && let Err(e) = s.record_success(&file_id, now)
                {
                    tracing::warn!("Failed to record AniDB success: {e}");
                }
            }
            Ok(LookupResult::NotFound) => {
                tracing::info!(%file_id, "AniDB: file not found");

                // Write None so clients know lookup was attempted
                let now = now_millis();
                let op = CrdtOp::LwwWrite {
                    timestamp: now,
                    value: LwwValue::AniDb(file_id, None),
                };
                let actions = sync_engine.lock().await.apply_local_op(op);
                dispatch_anidb_actions(actions, &clients, &storage, &sync_engine).await;

                // Record failure for revalidation scheduling
                if let Ok(s) = storage.lock()
                    && let Err(e) = s.record_failure(&file_id, now)
                {
                    tracing::warn!("Failed to record AniDB failure: {e}");
                }
            }
            Ok(LookupResult::Banned) => {
                tracing::error!("AniDB worker stopping: banned");
                return;
            }
            Err(e) => {
                tracing::warn!(%file_id, "AniDB lookup error: {e:#}");
                // Record failure for retry scheduling
                if let Ok(s) = storage.lock() {
                    let _ = s.record_failure(&file_id, now_millis());
                }
            }
        }
    }
}

/// Dispatch sync actions produced by the worker's CRDT operations.
///
/// This handles the same action types as the server's dispatch_sync_actions,
/// but from the worker context (no "from_client" to exclude).
async fn dispatch_anidb_actions(
    actions: Vec<SyncAction>,
    clients: &Arc<tokio::sync::RwLock<HashMap<PeerId, ConnectedClient>>>,
    storage: &Arc<std::sync::Mutex<ServerStorage>>,
    sync_engine: &Arc<tokio::sync::Mutex<SyncEngine>>,
) {
    for action in actions {
        match action {
            SyncAction::SendControl { peer, msg } => {
                let rv_msg = peer_control_to_rv(msg);
                if let Some(rv_msg) = rv_msg {
                    let cls = clients.read().await;
                    if let Some(client) = cls.get(&peer) {
                        let _ = client.control_tx.send(rv_msg);
                    }
                }
            }
            SyncAction::BroadcastControl { msg } => {
                let rv_msg = peer_control_to_rv(msg);
                if let Some(rv_msg) = rv_msg {
                    let cls = clients.read().await;
                    for client in cls.values() {
                        let _ = client.control_tx.send(rv_msg.clone());
                    }
                }
            }
            SyncAction::PersistOp { op } => {
                let epoch = sync_engine.lock().await.epoch();
                if let Ok(s) = storage.lock()
                    && let Err(e) = s.append_op(epoch, &op)
                {
                    tracing::warn!("AniDB worker: failed to persist op: {e}");
                }
            }
            SyncAction::PersistSnapshot { epoch, snapshot } => {
                if let Ok(s) = storage.lock()
                    && let Err(e) = s.save_snapshot(epoch, &snapshot)
                {
                    tracing::warn!("AniDB worker: failed to persist snapshot: {e}");
                }
            }
            // Worker doesn't send datagrams or do gap fill
            SyncAction::SendDatagram { .. }
            | SyncAction::BroadcastDatagram { .. }
            | SyncAction::RequestGapFill { .. } => {}
        }
    }
}

/// Convert a PeerControl message to RvControl for sending to clients.
fn peer_control_to_rv(msg: PeerControl) -> Option<RvControl> {
    match msg {
        PeerControl::StateOp { op } => Some(RvControl::StateOp { op }),
        PeerControl::StateSummary { versions, .. } => Some(RvControl::StateSummary { versions }),
        PeerControl::StateSnapshot { epoch, crdts } => {
            Some(RvControl::StateSnapshot { epoch, crdts })
        }
        _ => None,
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn peer_control_to_rv_state_op() {
        let op = CrdtOp::LwwWrite {
            timestamp: 100,
            value: LwwValue::AniDb(
                dessplay_core::types::FileId([0; 16]),
                None,
            ),
        };
        let msg = PeerControl::StateOp { op: op.clone() };
        let rv = peer_control_to_rv(msg).unwrap();
        assert!(matches!(rv, RvControl::StateOp { .. }));
    }

    #[test]
    fn peer_control_to_rv_hello_is_none() {
        let msg = PeerControl::Hello {
            peer_id: PeerId(1),
            username: "test".into(),
        };
        assert!(peer_control_to_rv(msg).is_none());
    }
}
