//! The tsugumi DB-restore incident, replayed (2026-08: the server's
//! database was restored from a backup; the epoch is a bare counter, so
//! the restore rolled it backwards — and the clients, which refuse any
//! snapshot below their own epoch, wedged in AwaitingSync forever with
//! their pre-restore state).
//!
//! These are the incident's two shapes:
//! - the backup's epoch is LOWER than the clients' — the client must
//!   adopt the restored snapshot anyway (the server is authoritative),
//!   not wedge;
//! - the backup lands on the SAME epoch the clients hold — the restored
//!   state must be adopted, not merged, or the union re-pollutes the
//!   very state the operator restored to clean up.

mod common;

use std::time::Duration;

use common::*;
use dessplay_core::types::Ed2kHash;
use dessplay_core::types::{ActorId, Epoch, SharedTimestamp};
use dessplay_core::{CrdtState, StateSnapshot, StateView};
use dessplay_rendezvous::server::ServerConfig;
use dessplay_rendezvous::storage::ServerStorage;

const BUDGET: Duration = Duration::from_secs(120);

/// An in-memory server DB holding `state` at `epoch` — the "backup" the
/// operator restores (or the pre-restore live DB).
fn storage_with(epoch: u64, state: &CrdtState) -> ServerStorage {
    let storage = ServerStorage::open_in_memory().expect("in-memory server storage");
    storage
        .save_state(
            &StateSnapshot {
                epoch: Epoch(epoch),
                state: state.clone(),
            },
            0,
        )
        .expect("seeding server state");
    storage
}

/// A state whose playlist is exactly `entries` (server-authored).
fn playlist_state(entries: &[u8]) -> CrdtState {
    let mut state = CrdtState::new();
    for (i, &e) in entries.iter().enumerate() {
        state.push_playlist_entry(ActorId::SERVER, SharedTimestamp(1 + i as u64), entry(e));
    }
    state
}

fn playlist_hashes(view: &StateView) -> Vec<Ed2kHash> {
    view.playlist.iter().map(|e| e.hash).collect()
}

/// The incident proper: the restored backup's epoch is LOWER than what
/// the clients hold. Every client must converge to the restored state
/// and epoch — the server is authoritative, and a connect-window
/// snapshot must be adopted regardless of its epoch. Pre-fix the client
/// refused the backward snapshot ("ignoring stale snapshot") and sat in
/// AwaitingSync forever, keeping its pre-restore state.
#[tokio::test(start_paused = true)]
async fn clients_converge_to_a_restored_lower_epoch_server() {
    init_test_logging();
    let live = playlist_state(&[1, 2, 3]);
    let mut harness = Harness::with_config_and_storage(
        0xA1,
        ServerConfig::new(PASSWORD),
        Some(storage_with(3, &live)),
    );
    let kim = harness.client("kim", 1);
    let dagger = harness.client("dagger", 2);
    eventually(&[&kim, &dagger], BUDGET, |snaps| {
        snaps
            .iter()
            .all(|s| s.epoch == Epoch(3) && playlist_hashes(&s.view) == [hash(1), hash(2), hash(3)])
    })
    .await;

    // The operator restores a backup taken before entry 3 existed —
    // and before two epoch bumps, so the counter rolls backwards.
    let backup = playlist_state(&[1, 2]);
    harness.restart_server(Some(storage_with(1, &backup)));

    eventually(&[&kim, &dagger], BUDGET, |snaps| {
        snaps
            .iter()
            .all(|s| s.epoch == Epoch(1) && playlist_hashes(&s.view) == [hash(1), hash(2)])
    })
    .await;
}

/// The collision shape: the restored backup lands on the SAME epoch the
/// clients hold. The restored state must be ADOPTED by reconnecting
/// clients, never merged with their pre-restore replicas — pre-fix the
/// equal epoch bought a StateMerge both ways, and the union quietly
/// re-polluted the server with exactly the entry the operator restored
/// the backup to remove. A fresh observer (which syncs whatever the
/// server holds) proves the pollution never returns server-side.
#[tokio::test(start_paused = true)]
async fn equal_epoch_restore_is_adopted_not_merged() {
    init_test_logging();
    // The backup is a genuine earlier copy of the live state: the
    // pollution (entry 9) was added on top of it.
    let clean = playlist_state(&[1, 2]);
    let mut polluted = clean.clone();
    polluted.push_playlist_entry(ActorId::SERVER, SharedTimestamp(50), entry(9));

    let mut harness = Harness::with_config_and_storage(
        0xB2,
        ServerConfig::new(PASSWORD),
        Some(storage_with(2, &polluted)),
    );
    let kim = harness.client("kim", 1);
    let dagger = harness.client("dagger", 2);
    eventually(&[&kim, &dagger], BUDGET, |snaps| {
        snaps
            .iter()
            .all(|s| s.epoch == Epoch(2) && playlist_hashes(&s.view) == [hash(1), hash(2), hash(9)])
    })
    .await;

    // Restore the pre-pollution backup; its epoch collides with the
    // clients' current one.
    harness.restart_server(Some(storage_with(2, &clean)));

    // The reconnecting clients adopt the restored state (dropping the
    // pollution) instead of merging it back in.
    eventually(&[&kim, &dagger], BUDGET, |snaps| {
        snaps
            .iter()
            .all(|s| playlist_hashes(&s.view) == [hash(1), hash(2)])
    })
    .await;

    // And the server itself stays clean: a fresh observer syncs the
    // server's state wholesale and must never see entry 9.
    let observer = harness.client("observer", 3);
    eventually(&[&observer], BUDGET, |snaps| {
        snaps
            .iter()
            .all(|s| playlist_hashes(&s.view) == [hash(1), hash(2)])
    })
    .await;
}
