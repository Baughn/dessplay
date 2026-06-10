//! Client composition: wires the network and sync actors together.
//!
//! This is the embryonic composition root (architecture.md): everything
//! here is constructible from a test with injected transport and clock.
//! Phase 5 grows it into the full `run_client` with UI, player, and
//! file actors; for now it provides a headless sync client — exactly
//! what the Phase 4 milestone needs.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use dessplay_core::net::Connector;
use dessplay_core::net::Role;
use dessplay_core::types::{ActorId, UserId};
use tokio::sync::mpsc;

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
pub fn spawn_client<C: Connector>(connector: Arc<C>, config: ClientConfig) -> ClientHandle {
    let epoch = Arc::new(AtomicU64::new(0));
    let actor = ActorId::session(&config.username.0, config.session_nonce);

    let (net_tx, net_rx) = mpsc::channel(64);
    let (net_event_tx, mut net_event_rx) = mpsc::channel(256);
    let (sync_tx, sync_rx) = mpsc::channel(256);
    let (sync_event_tx, mut sync_event_rx) = mpsc::channel(256);
    let (event_tx, event_rx) = mpsc::channel(256);

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

    let network_config = NetworkConfig::new(
        config.username,
        config.password,
        config.role,
        epoch,
        config.clock,
    );

    tokio::spawn(network::run(
        connector,
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
        while let Some(event) = net_event_rx.recv().await {
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
                NetworkEvent::PeerList(_) | NetworkEvent::AuthFailed => None,
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
    }
}
