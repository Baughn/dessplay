//! The local-report outbox handshake uses real sync persistence.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use dessplay::actors::sync::{self, SyncCommand, SyncConfig};
use dessplay::sync_storage::SyncStorage;
use dessplay_core::types::{ActorId, SharedTimestamp, UserId};
use std::sync::{Arc, atomic::AtomicU64};
use tokio::sync::{mpsc, oneshot};

#[tokio::test]
async fn roguelike_report_ack_is_durable_and_retries_survive_restart_without_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("client.sync.db");
    for session in 1..=2 {
        let storage = SyncStorage::open_at(&path).unwrap();
        let mut config = SyncConfig::new(
            UserId::new("kim"),
            ActorId::session("kim", session),
            Arc::new(|| 1000),
            Arc::new(AtomicU64::new(0)),
        );
        config.initial = storage.load_state().unwrap();
        config.storage = Some(storage);
        let (tx, rx) = mpsc::channel(16);
        let (net_tx, _net_rx) = mpsc::channel(16);
        let (events_tx, _events_rx) = mpsc::channel(16);
        let task = tokio::spawn(sync::run(config, rx, net_tx, events_tx));
        for _ in 0..3 {
            let (reply, received) = oneshot::channel();
            tx.send(SyncCommand::PublishLocalReport {
                text: "fell to a cave rat: floor 2, 80 turns [expedition #1]".into(),
                timestamp: SharedTimestamp(999),
                reply,
            })
            .await
            .unwrap();
            assert!(received.await.unwrap());
            // The acknowledgement must mean disk, even without a clean shutdown.
            let persisted = SyncStorage::open_at(&path)
                .unwrap()
                .load_state()
                .unwrap()
                .unwrap();
            assert_eq!(persisted.state.view().chat.len(), 1);
            assert_eq!(
                persisted.state.view().chat[0].timestamp,
                SharedTimestamp(999)
            );
        }
        // Simulate a crash, not the actor's graceful flush path.
        task.abort();
        let _ = task.await;
    }
}

#[tokio::test]
async fn roguelike_report_failed_flush_keeps_outbox_unacknowledged_and_retry_deduplicates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("client.sync.db");
    let storage = SyncStorage::open_at(&path).unwrap();
    let mut config = SyncConfig::new(
        UserId::new("kim"),
        ActorId::session("kim", 1),
        Arc::new(|| 1000),
        Arc::new(AtomicU64::new(0)),
    );
    config.storage = Some(storage);
    let fault = rusqlite::Connection::open(&path).unwrap();
    fault.execute_batch("CREATE TRIGGER fail_report_flush BEFORE INSERT ON crdt_state BEGIN SELECT RAISE(FAIL, 'disk write failed'); END;").unwrap();
    let (tx, rx) = mpsc::channel(16);
    let (net_tx, _net_rx) = mpsc::channel(16);
    let (events_tx, _events_rx) = mpsc::channel(16);
    let task = tokio::spawn(sync::run(config, rx, net_tx, events_tx));
    for success in [false, true] {
        if success {
            fault
                .execute_batch("DROP TRIGGER fail_report_flush;")
                .unwrap();
        }
        let (reply, received) = oneshot::channel();
        tx.send(SyncCommand::PublishLocalReport {
            text: "The Waiting Below: died [expedition #1]".into(),
            timestamp: SharedTimestamp(999),
            reply,
        })
        .await
        .unwrap();
        assert_eq!(received.await.unwrap(), success);
        let (reply, received) = oneshot::channel();
        tx.send(SyncCommand::GetView(reply)).await.unwrap();
        assert_eq!(received.await.unwrap().chat.len(), 1);
        let persisted = SyncStorage::open_at(&path).unwrap().load_state().unwrap();
        assert_eq!(persisted.is_some(), success);
    }
    task.abort();
    let _ = task.await;
}
