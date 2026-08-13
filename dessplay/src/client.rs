//! Client composition: wires the network and sync actors together.
//!
//! This is the embryonic composition root (architecture.md): everything
//! here is constructible from a test with injected transport and clock.
//! Phase 5 grows it into the full `run_client` with UI, player, and
//! file actors; for now it provides a headless sync client — exactly
//! what the Phase 4 milestone needs.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use dessplay_core::net::{Connector, KnownUser, PeerInfo, Role};
use dessplay_core::types::{ActorId, UserId};
use tokio::sync::{mpsc, watch};

use crate::actors::network::{self, Clock, NetworkConfig, NetworkEvent};
use crate::actors::sync::{self, SyncCommand, SyncConfig, SyncEvent};

/// Events surfaced to the owner (tests now, the UI layer in Phase 5+).
#[derive(Debug)]
pub enum ClientEvent {
    /// Connection-level events (connected, peer lists, disconnects...).
    Network(NetworkEvent),
    /// State-level events (state changed, divergence alarm).
    Sync(SyncEvent),
}

/// Handles to a running headless client.
pub struct ClientHandle {
    /// Send mutations and queries to the sync actor.
    pub sync: mpsc::Sender<SyncCommand>,
    /// Send network commands (rarely needed directly; `Shutdown`).
    pub network: mpsc::Sender<network::NetworkCommand>,
    /// Everything the client wants you to know.
    pub events: mpsc::Receiver<ClientEvent>,
    /// The latest peer list (presence included) — one input to the
    /// derived playback state (`dessplay_core::derive`). Borrow, don't
    /// consume: `peers.borrow().clone()`. Includes synthesized Departed
    /// entries for recently-seen known-offline users
    /// (`derive::merge_known_offline`), so a committed user still gates
    /// across a server restart; the raw server list is only on the
    /// surfaced `NetworkEvent::PeerList`.
    pub peers: watch::Receiver<Vec<PeerInfo>>,
    /// Known usernames not currently in `peers` (design.md #15). Borrow,
    /// don't consume, same as `peers`.
    pub known_offline: watch::Receiver<Vec<KnownUser>>,
    /// The latest connection-health sample: `None` before the first
    /// report and after a disconnect. A watch, not an event — the 1Hz
    /// samples are latest-value metrics and must never compete with
    /// reliable events for channel capacity (a slow consumer would
    /// otherwise stall the router, and with it state sync, behind
    /// droppable numbers).
    pub health: watch::Receiver<Option<network::LinkHealthReport>>,
}

/// Configuration for a headless sync client.
pub struct ClientConfig {
    /// Our username.
    pub username: UserId,
    /// Shared room password.
    pub password: String,
    /// Interactive or seeder.
    pub role: Role,
    /// Session nonce for the actor id (chosen by the caller — random in
    /// production, fixed in tests).
    pub session_nonce: u128,
    /// Local clock, unix millis.
    pub clock: Clock,
    /// Sync actor extras (initial snapshot, storage, flush cadence).
    pub sync: SyncConfigExtras,
}

/// The non-identity parts of [`SyncConfig`].
#[derive(Default)]
pub struct SyncConfigExtras {
    /// Stored snapshot to start from.
    pub initial: Option<dessplay_core::StateSnapshot>,
    /// Persistence; `None` runs stateless.
    pub storage: Option<crate::storage::Storage>,
    /// Flush cadence override (default 30s).
    pub flush_interval: Option<std::time::Duration>,
}

/// Spawn the headless client: network actor + sync actor + router.
///
/// `transfer_connector` dials the transfer connection (see the network
/// actor's docs) — in production the control address at port + 1 with
/// the bulk DSCP tag, in tests a second sim connector.
pub fn spawn_client<C: Connector>(
    connector: Arc<C>,
    transfer_connector: Arc<C>,
    config: ClientConfig,
) -> ClientHandle {
    let epoch = Arc::new(AtomicU64::new(0));
    let actor = ActorId::session(&config.username.0, config.session_nonce);

    let (net_tx, net_rx) = mpsc::channel(64);
    let (net_event_tx, mut net_event_rx) = mpsc::channel(256);
    let (sync_tx, sync_rx) = mpsc::channel(256);
    let (sync_event_tx, mut sync_event_rx) = mpsc::channel(256);
    let (event_tx, event_rx) = mpsc::channel(256);
    let (peers_tx, peers_rx) = watch::channel(Vec::new());
    let (known_offline_tx, known_offline_rx) = watch::channel(Vec::new());
    let (health_tx, health_rx) = watch::channel(None);

    let mut sync_config = SyncConfig::new(
        config.username.clone(),
        actor,
        Arc::clone(&config.clock),
        Arc::clone(&epoch),
    );
    sync_config.initial = config.sync.initial;
    sync_config.storage = config.sync.storage;
    if let Some(interval) = config.sync.flush_interval {
        sync_config.flush_interval = interval;
    }

    let router_clock = Arc::clone(&config.clock);
    let network_config = NetworkConfig::new(
        config.username,
        config.password,
        config.role,
        epoch,
        config.clock,
    );

    tokio::spawn(network::run(
        connector,
        transfer_connector,
        network_config,
        net_rx,
        net_event_tx,
    ));
    tokio::spawn(sync::run(
        sync_config,
        sync_rx,
        net_tx.clone(),
        sync_event_tx,
    ));

    // Router: network events fan out to the sync actor and the owner.
    let router_sync = sync_tx.clone();
    let router_events = event_tx.clone();
    tokio::spawn(async move {
        // Shared-clock offset, tracked so the known-offline gating horizon
        // is measured against the server's clock domain (`last_seen` is
        // server-written). Before the first ClockSync it is 0 — local
        // time, close enough for a 7-day horizon.
        let mut clock_offset: i64 = 0;
        while let Some(event) = net_event_rx.recv().await {
            // Health samples land in the watch and go no further — see
            // the `health` field's doc for why they are not events.
            if let NetworkEvent::LinkHealth(report) = &event {
                let _ = health_tx.send(Some(*report));
                continue;
            }
            if matches!(event, NetworkEvent::Disconnected { .. }) {
                // Stale health must not outlive the connection that
                // measured it.
                let _ = health_tx.send(None);
            }
            if let NetworkEvent::ClockSync { offset_millis } = &event {
                clock_offset = *offset_millis;
            }
            if let NetworkEvent::PeerList {
                peers,
                known_offline,
            } = &event
            {
                // Known-offline users seen within the last week gate like
                // Departed peers (design.md #15/Presence): without this a
                // server restart empties the registry and silently waives
                // every absent user's commitment.
                let now = ((router_clock)() as i64 + clock_offset).max(0) as u64;
                let merged = dessplay_core::derive::merge_known_offline(peers, known_offline, now);
                let _ = peers_tx.send(merged);
                let _ = known_offline_tx.send(known_offline.clone());
            }
            let to_sync = match &event {
                NetworkEvent::Server { msg, via_datagram } => Some(SyncCommand::Server {
                    msg: msg.clone(),
                    via_datagram: *via_datagram,
                }),
                NetworkEvent::Connected { .. } => Some(SyncCommand::Connected),
                NetworkEvent::Disconnected { .. } => Some(SyncCommand::Disconnected),
                NetworkEvent::ClockSync { offset_millis } => Some(SyncCommand::ClockSync {
                    offset_millis: *offset_millis,
                }),
                NetworkEvent::PeerList { .. }
                | NetworkEvent::Rejected { .. }
                | NetworkEvent::SearchResults { .. }
                | NetworkEvent::Connecting { .. }
                | NetworkEvent::LinkHealth(_)
                | NetworkEvent::Peer { .. }
                | NetworkEvent::TransferStream { .. }
                | NetworkEvent::TransferStreamFailed { .. } => None,
            };
            if let Some(cmd) = to_sync
                && router_sync.send(cmd).await.is_err()
            {
                break;
            }
            // State-sync payloads are the sync actor's business only;
            // everything else is surfaced.
            if !matches!(event, NetworkEvent::Server { .. })
                && router_events
                    .send(ClientEvent::Network(event))
                    .await
                    .is_err()
            {
                break;
            }
        }
    });

    // Sync events surface directly.
    tokio::spawn(async move {
        while let Some(event) = sync_event_rx.recv().await {
            if event_tx.send(ClientEvent::Sync(event)).await.is_err() {
                break;
            }
        }
    });

    ClientHandle {
        sync: sync_tx,
        network: net_tx,
        events: event_rx,
        peers: peers_rx,
        known_offline: known_offline_rx,
        health: health_rx,
    }
}
