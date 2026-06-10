//! The headless multi-client harness (testing-strategy.md): N real
//! clients (network + sync actors) against the real server over the
//! simulated transport, in one paused-time runtime. Phase 6 adds UI
//! handles, Phase 7 player handles.

#![allow(dead_code)] // each test binary uses a subset

use std::sync::Arc;
use std::time::Duration;

use dessplay::actors::sync::{Mutation, SyncCommand};
use dessplay::client::{ClientConfig, ClientHandle, SyncConfigExtras, spawn_client};
use dessplay_core::net::sim::{EndpointId, SimNetwork};
use dessplay_core::net::{PeerInfo, Role, ServerControl};
use dessplay_core::playlist::NewPlaylistEntry;
use dessplay_core::types::{Ed2kHash, Epoch, UserId};
use dessplay_core::{StateView, derive};
use dessplay_rendezvous::server::{self, ServerConfig};
use tokio::sync::oneshot;

pub const PASSWORD: &str = "hunter2";

/// A clock that follows paused tokio time from a fixed origin.
pub fn sim_clock(skew_millis: i64) -> Arc<dyn Fn() -> u64 + Send + Sync> {
    let origin = tokio::time::Instant::now();
    Arc::new(move || {
        let elapsed = tokio::time::Instant::now().duration_since(origin);
        (1_700_000_000_000_i64 + elapsed.as_millis() as i64 + skew_millis) as u64
    })
}

/// The harness: a sim network with the real server listening on it.
pub struct Harness {
    pub net: SimNetwork,
    pub server_id: EndpointId,
}

impl Harness {
    /// Server with default config (password only).
    pub fn new(seed: u64) -> Self {
        Self::with_config(seed, ServerConfig::new(PASSWORD))
    }

    /// Server with a custom config (compaction schedules, chat_keep).
    pub fn with_config(seed: u64, config: ServerConfig) -> Self {
        let net = SimNetwork::new(seed);
        let server_id = EndpointId::new("server");
        let listener = net.listener(&server_id);
        tokio::spawn(server::run(listener, config, sim_clock(0), None));
        Self { net, server_id }
    }

    /// Spawn a full headless client.
    pub fn client(&self, name: &str, nonce: u128) -> ClientHandle {
        self.client_with_role(name, nonce, Role::Interactive)
    }

    /// Spawn a seeder.
    pub fn seeder(&self, name: &str, nonce: u128) -> ClientHandle {
        self.client_with_role(name, nonce, Role::Seeder)
    }

    fn client_with_role(&self, name: &str, nonce: u128, role: Role) -> ClientHandle {
        let connector = Arc::new(self.net.connector(&EndpointId::new(name), &self.server_id));
        spawn_client(
            connector,
            ClientConfig {
                username: UserId::new(name),
                password: PASSWORD.into(),
                role,
                session_nonce: nonce,
                clock: sim_clock(0),
                sync: SyncConfigExtras::default(),
            },
        )
    }

    /// Cut a client's connection *and* block reconnection attempts, so
    /// the user actually stays Lost (the network actor retries every
    /// 2s; without the partition it would be back within seconds).
    pub fn isolate(&self, name: &str) {
        self.net
            .set_partitioned(&EndpointId::new(name), &self.server_id, true);
        self.net.disconnect(&EndpointId::new(name), &self.server_id);
    }

    /// Allow an isolated client to reconnect.
    pub fn heal(&self, name: &str) {
        self.net
            .set_partitioned(&EndpointId::new(name), &self.server_id, false);
    }
}

/// One client's observable world: resolved view + latest peer list +
/// epoch.
#[derive(Debug)]
pub struct ClientSnapshot {
    pub view: StateView,
    pub peers: Vec<PeerInfo>,
    pub epoch: Epoch,
}

impl ClientSnapshot {
    /// The derived playback state as this client computes it.
    pub fn playing(&self) -> bool {
        derive::playback_active(&self.view, &self.peers)
    }

    /// This client's record of a peer, if listed.
    pub fn peer(&self, name: &str) -> Option<&PeerInfo> {
        self.peers.iter().find(|p| p.username == UserId::new(name))
    }
}

pub async fn view_of(handle: &ClientHandle) -> StateView {
    let (tx, rx) = oneshot::channel();
    handle.sync.send(SyncCommand::GetView(tx)).await.unwrap();
    rx.await.unwrap()
}

pub async fn epoch_of(handle: &ClientHandle) -> Epoch {
    let (tx, rx) = oneshot::channel();
    handle.sync.send(SyncCommand::GetEpoch(tx)).await.unwrap();
    rx.await.unwrap()
}

pub async fn snapshot_of(handle: &ClientHandle) -> ClientSnapshot {
    ClientSnapshot {
        view: view_of(handle).await,
        peers: handle.peers.borrow().clone(),
        epoch: epoch_of(handle).await,
    }
}

pub async fn mutate(handle: &ClientHandle, mutation: Mutation) {
    handle
        .sync
        .send(SyncCommand::Mutate(Box::new(mutation)))
        .await
        .unwrap();
}

/// Report end-of-file to the server, as the player layer will.
pub async fn report_eof(handle: &ClientHandle, file: Ed2kHash) {
    handle
        .network
        .send(dessplay::actors::network::NetworkCommand::SendReliable(
            Box::new(ServerControl::EofReached { file }),
        ))
        .await
        .unwrap();
}

/// Graceful quit: Goodbye + connection teardown.
pub async fn quit(handle: &ClientHandle) {
    handle
        .network
        .send(dessplay::actors::network::NetworkCommand::Shutdown)
        .await
        .unwrap();
}

/// Wait (in simulated time) until `pred` holds over all clients'
/// snapshots. The auto-waiting assertion from testing-strategy.md.
pub async fn eventually<F: FnMut(&[ClientSnapshot]) -> bool>(
    clients: &[&ClientHandle],
    budget: Duration,
    mut pred: F,
) -> Vec<ClientSnapshot> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let mut snapshots = Vec::new();
        for client in clients {
            snapshots.push(snapshot_of(client).await);
        }
        if pred(&snapshots) {
            return snapshots;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("condition not reached; final snapshots: {snapshots:#?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Like [`eventually`], but over views only (convergence checks).
pub async fn eventually_views<F: FnMut(&[StateView]) -> bool>(
    clients: &[&ClientHandle],
    budget: Duration,
    mut pred: F,
) -> Vec<StateView> {
    eventually(clients, budget, |snapshots| {
        let views: Vec<StateView> = snapshots.iter().map(|s| s.view.clone()).collect();
        pred(&views)
    })
    .await
    .into_iter()
    .map(|s| s.view)
    .collect()
}

/// A deterministic playlist entry.
pub fn entry(i: u8) -> NewPlaylistEntry {
    NewPlaylistEntry {
        hash: Ed2kHash([i; 16]),
        added_by: UserId::new("whoever"),
        filename: format!("ep{i}.mkv"),
        size_bytes: 1_000_000,
        duration_millis: Some(1_440_000),
    }
}

pub fn hash(i: u8) -> Ed2kHash {
    Ed2kHash([i; 16])
}
