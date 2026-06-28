//! Connection-flow tests: the real client NetworkActor against the real
//! server, over the simulated transport, under paused time. This is the
//! first cross-crate slice of the multi-client harness.

use std::sync::Arc;
use std::time::Duration;

use dessplay::actors::network::{self, NetworkCommand, NetworkConfig, NetworkEvent};
use dessplay_core::net::sim::{EndpointId, LinkConfig, SimNetwork};
use dessplay_core::net::{Presence, Role};
use dessplay_core::types::UserId;
use dessplay_rendezvous::server::{self, ServerConfig};
use std::sync::atomic::AtomicU64;
use tokio::sync::mpsc;

const PASSWORD: &str = "hunter2";

/// A clock derived from paused tokio time plus a fixed skew —
/// deterministic, and skew lets time-sync accuracy be asserted exactly.
fn sim_clock(skew_millis: i64) -> Arc<dyn Fn() -> u64 + Send + Sync> {
    let origin = tokio::time::Instant::now();
    Arc::new(move || {
        let elapsed = tokio::time::Instant::now().duration_since(origin);
        (1_700_000_000_000_i64 + elapsed.as_millis() as i64 + skew_millis) as u64
    })
}

struct TestClient {
    events: mpsc::Receiver<NetworkEvent>,
    commands: mpsc::Sender<NetworkCommand>,
}

fn spawn_client(
    net: &SimNetwork,
    name: &str,
    server_id: &EndpointId,
    password: &str,
    role: Role,
    clock_skew: i64,
) -> TestClient {
    let connector = Arc::new(net.connector(&EndpointId::new(name), server_id));
    let (command_tx, command_rx) = mpsc::channel(8);
    let (event_tx, event_rx) = mpsc::channel(64);
    let config = NetworkConfig {
        time_sync_interval: Duration::from_secs(30),
        reconnect_backoff: Duration::from_secs(2),
        ..NetworkConfig::new(
            UserId::new(name),
            password.into(),
            role,
            Arc::new(AtomicU64::new(0)),
            sim_clock(clock_skew),
        )
    };
    tokio::spawn(network::run(connector, config, command_rx, event_tx));
    TestClient {
        events: event_rx,
        commands: command_tx,
    }
}

/// Receive events until `pred` returns Some, with a simulated-time
/// budget. The `eventually` of the future harness, in embryonic form.
async fn expect_event<T>(
    client: &mut TestClient,
    budget: Duration,
    mut pred: impl FnMut(&NetworkEvent) -> Option<T>,
) -> T {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let event = tokio::time::timeout_at(deadline, client.events.recv())
            .await
            .expect("event budget exhausted")
            .expect("event channel closed");
        if let Some(out) = pred(&event) {
            return out;
        }
    }
}

fn setup() -> (SimNetwork, EndpointId) {
    let net = SimNetwork::new(0xD355);
    let server_id = EndpointId::new("server");
    let listener = net.listener(&server_id);
    tokio::spawn(server::run(
        listener,
        ServerConfig::new(PASSWORD),
        sim_clock(0),
        None,
    ));
    (net, server_id)
}

#[tokio::test(start_paused = true)]
async fn connect_auth_peerlist_and_clock() {
    let (net, server_id) = setup();

    // Give the link some latency so time sync has something to chew on.
    net.set_link(
        &EndpointId::new("baughn"),
        &server_id,
        LinkConfig {
            latency: Duration::from_millis(40),
            ..LinkConfig::default()
        },
    );

    // Client clock runs 5s behind the server.
    let mut client = spawn_client(
        &net,
        "baughn",
        &server_id,
        PASSWORD,
        Role::Interactive,
        -5_000,
    );

    expect_event(&mut client, Duration::from_secs(5), |e| {
        matches!(e, NetworkEvent::Connected { .. }).then_some(())
    })
    .await;

    // Peer list contains us, Present, Interactive.
    expect_event(&mut client, Duration::from_secs(5), |e| match e {
        NetworkEvent::PeerList(peers) => peers
            .iter()
            .any(|p| {
                p.username == UserId::new("baughn")
                    && p.role == Role::Interactive
                    && p.presence == Presence::Present
            })
            .then_some(()),
        _ => None,
    })
    .await;

    // The clock offset converges to exactly +5000 (symmetric latency).
    let offset = expect_event(&mut client, Duration::from_secs(10), |e| match e {
        NetworkEvent::ClockSync { offset_millis } => Some(*offset_millis),
        _ => None,
    })
    .await;
    assert_eq!(offset, 5_000);
}

#[tokio::test(start_paused = true)]
async fn second_client_appears_in_both_peer_lists() {
    let (net, server_id) = setup();
    let mut kim = spawn_client(&net, "kim", &server_id, PASSWORD, Role::Interactive, 0);
    expect_event(&mut kim, Duration::from_secs(5), |e| {
        matches!(e, NetworkEvent::Connected { .. }).then_some(())
    })
    .await;

    let mut nas = spawn_client(&net, "nas", &server_id, PASSWORD, Role::Seeder, 0);
    expect_event(&mut nas, Duration::from_secs(5), |e| {
        matches!(e, NetworkEvent::Connected { .. }).then_some(())
    })
    .await;

    // Both eventually see a two-peer list with the right roles.
    for client in [&mut kim, &mut nas] {
        expect_event(client, Duration::from_secs(5), |e| match e {
            NetworkEvent::PeerList(peers) if peers.len() == 2 => {
                let kim_ok = peers
                    .iter()
                    .any(|p| p.username == UserId::new("kim") && p.role == Role::Interactive);
                let nas_ok = peers
                    .iter()
                    .any(|p| p.username == UserId::new("nas") && p.role == Role::Seeder);
                (kim_ok && nas_ok).then_some(())
            }
            _ => None,
        })
        .await;
    }
}

#[tokio::test(start_paused = true)]
async fn bad_password_is_rejected() {
    let (net, server_id) = setup();
    let mut client = spawn_client(&net, "mallory", &server_id, "wrong", Role::Interactive, 0);
    expect_event(&mut client, Duration::from_secs(5), |e| {
        matches!(e, NetworkEvent::AuthFailed).then_some(())
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn duplicate_username_supersedes_old_connection() {
    let (net, server_id) = setup();
    let mut first = spawn_client(&net, "kim", &server_id, PASSWORD, Role::Interactive, 0);
    expect_event(&mut first, Duration::from_secs(5), |e| {
        matches!(e, NetworkEvent::Connected { .. }).then_some(())
    })
    .await;

    // Same username, fresh "device". The sim needs a distinct endpoint
    // id, so connect from another endpoint name but auth as kim.
    let connector = Arc::new(net.connector(&EndpointId::new("kim-laptop"), &server_id));
    let (_cmd_tx, cmd_rx) = mpsc::channel(8);
    let (event_tx, event_rx) = mpsc::channel(64);
    tokio::spawn(network::run(
        connector,
        NetworkConfig::new(
            UserId::new("kim"),
            PASSWORD.into(),
            Role::Interactive,
            Arc::new(AtomicU64::new(0)),
            sim_clock(0),
        ),
        cmd_rx,
        event_tx,
    ));
    let mut second = TestClient {
        events: event_rx,
        commands: mpsc::channel(1).0,
    };

    expect_event(&mut second, Duration::from_secs(5), |e| {
        matches!(e, NetworkEvent::Connected { .. }).then_some(())
    })
    .await;

    // The first connection gets cut with the supersede reason and
    // recycles into a reconnect attempt (which will in turn supersede
    // the second — that ping-pong is fine; we just assert the cut).
    expect_event(&mut first, Duration::from_secs(10), |e| match e {
        NetworkEvent::Disconnected { reason } if reason.contains("superseded") => Some(()),
        _ => None,
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn graceful_shutdown_updates_peer_list() {
    let (net, server_id) = setup();
    let mut kim = spawn_client(&net, "kim", &server_id, PASSWORD, Role::Interactive, 0);
    let mut dag = spawn_client(&net, "dagger", &server_id, PASSWORD, Role::Interactive, 0);
    for client in [&mut kim, &mut dag] {
        expect_event(client, Duration::from_secs(5), |e| {
            matches!(e, NetworkEvent::Connected { .. }).then_some(())
        })
        .await;
    }
    // Wait until dagger has seen the 2-peer world.
    expect_event(&mut dag, Duration::from_secs(5), |e| match e {
        NetworkEvent::PeerList(peers) if peers.len() == 2 => Some(()),
        _ => None,
    })
    .await;

    kim.commands.send(NetworkCommand::Shutdown).await.unwrap();

    // Dagger sees kim depart *in place*: still listed, now Departed (a
    // clean quit is an immediate departure, not a registry removal).
    expect_event(&mut dag, Duration::from_secs(10), |e| match e {
        NetworkEvent::PeerList(peers) => {
            let kim = peers.iter().find(|p| p.username == UserId::new("kim"));
            let dagger = peers.iter().find(|p| p.username == UserId::new("dagger"));
            (kim.is_some_and(|p| p.presence == Presence::Departed)
                && dagger.is_some_and(|p| p.presence == Presence::Present))
            .then_some(())
        }
        _ => None,
    })
    .await;
}
