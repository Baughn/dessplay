//! Integration tests for the network layer.
//!
//! These tests start a real rendezvous server on a random localhost port and
//! connect real QUIC clients to it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use quinn::Endpoint;

use dessplay_core::framing::{read_framed, write_framed, TAG_RV_CONTROL};
use dessplay_core::network::NetworkEvent;
use dessplay_core::protocol::{PeerControl, PeerInfo, RvControl};
use dessplay_core::types::PeerId;

use dessplay::peer_conn::PeerManager;
use dessplay::rendezvous_client::{RendezvousClient, RendezvousEvent};
use dessplay::tls::AcceptAnyCert;

/// Install the rustls crypto provider (must be called once per test process).
fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Helper: start a rendezvous server on a random localhost port.
/// Returns the server endpoint's local address.
async fn start_test_server(password: &str) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    ensure_crypto_provider();
    // Generate ephemeral cert for the server
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();

    let cert_der = rustls::pki_types::CertificateDer::from(cert.der().to_vec());
    let key_der = rustls::pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der());

    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der.into())
        .unwrap();
    server_crypto.alpn_protocols = vec![b"dessplay".to_vec()];

    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto).unwrap(),
    ));

    // Bind to [::1]:0 for a random port
    let bind: SocketAddr = "[::1]:0".parse().unwrap();
    let endpoint = Endpoint::server(server_config, bind).unwrap();
    let addr = endpoint.local_addr().unwrap();

    let server =
        dessplay_rendezvous::server::RendezvousServer::new(endpoint, password.to_string());
    let handle = tokio::spawn(async move {
        let _ = server.run().await;
    });

    (addr, handle)
}

/// Helper: create a test client endpoint that trusts any certificate (client-only).
fn create_test_client() -> Endpoint {
    let mut client_crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
        .with_no_client_auth();
    client_crypto.alpn_protocols = vec![b"dessplay".to_vec()];

    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto).unwrap(),
    ));

    let bind: SocketAddr = "[::1]:0".parse().unwrap();
    let mut endpoint = Endpoint::client(bind).unwrap();
    endpoint.set_default_client_config(client_config);
    endpoint
}

/// Helper: create a dual-mode test endpoint (server + accept-any-cert client).
/// Returns the endpoint and the peer client config for connect_with().
fn create_test_dual_endpoint() -> (Endpoint, quinn::ClientConfig) {
    ensure_crypto_provider();

    // Generate ephemeral cert for server side
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["dessplay".to_string()]).unwrap();
    let cert_der = rustls::pki_types::CertificateDer::from(cert.der().to_vec());
    let key_der = rustls::pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der());

    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der.into())
        .unwrap();
    server_crypto.alpn_protocols = vec![b"dessplay".to_vec()];

    let server_config = quinn::ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto).unwrap(),
    ));

    // Accept-any-cert client config
    let mut client_crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
        .with_no_client_auth();
    client_crypto.alpn_protocols = vec![b"dessplay".to_vec()];

    let peer_client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto).unwrap(),
    ));

    let bind: SocketAddr = "[::1]:0".parse().unwrap();
    let mut endpoint = Endpoint::server(server_config, bind).unwrap();
    endpoint.set_default_client_config(peer_client_config.clone());

    (endpoint, peer_client_config)
}

/// Connect a test client, authenticate, and return the connection + streams + peer_id.
async fn connect_and_auth(
    endpoint: &Endpoint,
    server_addr: SocketAddr,
    password: &str,
    username: &str,
) -> Result<(
    quinn::Connection,
    quinn::SendStream,
    quinn::RecvStream,
    PeerId,
    SocketAddr,
)> {
    let conn = endpoint.connect(server_addr, "localhost")?.await?;
    let (mut send, mut recv) = conn.open_bi().await?;

    write_framed(
        &mut send,
        TAG_RV_CONTROL,
        &RvControl::Auth {
            password: password.to_string(),
            username: username.to_string(),
        },
    )
    .await?;

    let response: RvControl = read_framed(&mut recv, TAG_RV_CONTROL)
        .await?
        .expect("expected auth response");

    match response {
        RvControl::AuthOk {
            peer_id,
            observed_addr,
        } => Ok((conn, send, recv, peer_id, observed_addr)),
        RvControl::AuthFailed => Err(anyhow::anyhow!("auth failed")),
        other => Err(anyhow::anyhow!("unexpected: {other:?}")),
    }
}

/// Wait for a PeerList with at least `count` peers from a RendezvousClient.
async fn wait_for_peer_list(rv: &RendezvousClient, count: usize) -> Vec<PeerInfo> {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), rv.recv())
            .await
            .expect("timeout waiting for peer list")
            .expect("rendezvous client closed");
        match event {
            RendezvousEvent::PeerList { peers } if peers.len() >= count => return peers,
            _ => continue,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auth_success() {
    let (addr, _server) = start_test_server("testpw").await;
    let client = create_test_client();

    let (_conn, _send, _recv, peer_id, observed_addr) =
        connect_and_auth(&client, addr, "testpw", "alice").await.unwrap();

    assert_eq!(peer_id, PeerId(1));
    assert!(!observed_addr.ip().is_unspecified());
}

#[tokio::test]
async fn auth_failure() {
    let (addr, _server) = start_test_server("correctpw").await;
    let client = create_test_client();

    let conn = client.connect(addr, "localhost").unwrap().await.unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();

    write_framed(
        &mut send,
        TAG_RV_CONTROL,
        &RvControl::Auth {
            password: "wrongpw".to_string(),
            username: "alice".to_string(),
        },
    )
    .await
    .unwrap();

    // Server sends AuthFailed then drops the connection. We may receive
    // either the AuthFailed message or a connection-closed error.
    match read_framed::<_, RvControl>(&mut recv, TAG_RV_CONTROL).await {
        Ok(Some(RvControl::AuthFailed)) => {} // expected
        Ok(Some(other)) => panic!("expected AuthFailed, got {other:?}"),
        Ok(None) => {} // server closed cleanly
        Err(_) => {}   // connection dropped before we could read
    }
}

#[tokio::test]
async fn time_sync_accuracy() {
    let (addr, _server) = start_test_server("pw").await;
    let client = create_test_client();

    let (_conn, mut send, mut recv, _peer_id, _) =
        connect_and_auth(&client, addr, "pw", "alice").await.unwrap();

    // Drain the initial PeerList
    let _: Option<RvControl> = read_framed(&mut recv, TAG_RV_CONTROL).await.unwrap();

    // Send time sync request
    let t1 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    write_framed(
        &mut send,
        TAG_RV_CONTROL,
        &RvControl::TimeSyncRequest { client_send: t1 },
    )
    .await
    .unwrap();

    let response: RvControl = read_framed(&mut recv, TAG_RV_CONTROL)
        .await
        .unwrap()
        .expect("expected time sync response");

    match response {
        RvControl::TimeSyncResponse {
            client_send,
            server_recv,
            server_send,
        } => {
            assert_eq!(client_send, t1);
            // Server timestamps should be reasonable (within 1s of client)
            let diff_recv = (server_recv as i64 - t1 as i64).unsigned_abs();
            let diff_send = (server_send as i64 - t1 as i64).unsigned_abs();
            assert!(
                diff_recv < 1000,
                "server_recv too far from client: {diff_recv}ms"
            );
            assert!(
                diff_send < 1000,
                "server_send too far from client: {diff_send}ms"
            );
        }
        other => panic!("expected TimeSyncResponse, got {other:?}"),
    }
}

#[tokio::test]
async fn peer_discovery() {
    let (addr, _server) = start_test_server("pw").await;

    let client_a = create_test_client();
    let client_b = create_test_client();

    // Connect Alice
    let (_conn_a, _send_a, mut recv_a, peer_id_a, _) =
        connect_and_auth(&client_a, addr, "pw", "alice").await.unwrap();

    // Drain Alice's initial peer list (just herself)
    let msg: RvControl = read_framed(&mut recv_a, TAG_RV_CONTROL)
        .await
        .unwrap()
        .expect("expected peer list");
    match &msg {
        RvControl::PeerList { peers } => {
            assert_eq!(peers.len(), 1);
            assert_eq!(peers[0].username, "alice");
        }
        other => panic!("expected PeerList, got {other:?}"),
    }

    // Connect Bob
    let (_conn_b, _send_b, mut recv_b, peer_id_b, _) =
        connect_and_auth(&client_b, addr, "pw", "bob").await.unwrap();

    assert_ne!(peer_id_a, peer_id_b);

    // Alice should receive an updated PeerList with both peers
    let msg: RvControl = read_framed(&mut recv_a, TAG_RV_CONTROL)
        .await
        .unwrap()
        .expect("expected updated peer list");
    match &msg {
        RvControl::PeerList { peers } => {
            assert_eq!(peers.len(), 2);
            let usernames: Vec<&str> = peers.iter().map(|p| p.username.as_str()).collect();
            assert!(usernames.contains(&"alice"));
            assert!(usernames.contains(&"bob"));
        }
        other => panic!("expected PeerList, got {other:?}"),
    }

    // Bob should receive a PeerList with both peers
    let msg: RvControl = read_framed(&mut recv_b, TAG_RV_CONTROL)
        .await
        .unwrap()
        .expect("expected peer list for bob");
    match &msg {
        RvControl::PeerList { peers } => {
            assert_eq!(peers.len(), 2);
        }
        other => panic!("expected PeerList, got {other:?}"),
    }
}

#[tokio::test]
async fn peer_disconnect_updates_list() {
    let (addr, _server) = start_test_server("pw").await;

    let client_a = create_test_client();
    let client_b = create_test_client();

    // Connect both
    let (_conn_a, _send_a, mut recv_a, _, _) =
        connect_and_auth(&client_a, addr, "pw", "alice").await.unwrap();
    let (conn_b, _send_b, _recv_b, _, _) =
        connect_and_auth(&client_b, addr, "pw", "bob").await.unwrap();

    // Drain Alice's peer list messages (initial + bob joined)
    let _ = read_framed::<_, RvControl>(&mut recv_a, TAG_RV_CONTROL).await;
    let _ = read_framed::<_, RvControl>(&mut recv_a, TAG_RV_CONTROL).await;

    // Disconnect Bob
    conn_b.close(0u32.into(), b"bye");
    // Give server time to detect and broadcast
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Alice should get an updated peer list with only herself
    let msg = tokio::time::timeout(
        Duration::from_secs(2),
        read_framed::<_, RvControl>(&mut recv_a, TAG_RV_CONTROL),
    )
    .await
    .expect("timeout waiting for updated peer list")
    .unwrap()
    .expect("expected peer list");

    match &msg {
        RvControl::PeerList { peers } => {
            assert_eq!(peers.len(), 1);
            assert_eq!(peers[0].username, "alice");
        }
        other => panic!("expected PeerList, got {other:?}"),
    }
}

#[tokio::test]
async fn multiple_peer_ids_are_unique() {
    let (addr, _server) = start_test_server("pw").await;

    let mut peer_ids = Vec::new();
    for i in 0..5 {
        let client = create_test_client();
        let (_, _, _, peer_id, _) = connect_and_auth(
            &client,
            addr,
            "pw",
            &format!("user{i}"),
        )
        .await
        .unwrap();
        peer_ids.push(peer_id);
    }

    // All peer IDs should be unique
    let mut sorted = peer_ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), peer_ids.len(), "peer IDs should be unique");
}

// ---------------------------------------------------------------------------
// Peer-to-Peer Milestone Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn peer_to_peer_connection() {
    // Start rendezvous server
    let (server_addr, _server) = start_test_server("pw").await;

    // Create dual endpoints for two peers
    let (endpoint_a, peer_cfg_a) = create_test_dual_endpoint();
    let (endpoint_b, peer_cfg_b) = create_test_dual_endpoint();

    // Connect both to rendezvous server
    let rv_a = RendezvousClient::connect(&endpoint_a, server_addr, "localhost", "pw", "alice")
        .await
        .unwrap();
    let rv_b = RendezvousClient::connect(&endpoint_b, server_addr, "localhost", "pw", "bob")
        .await
        .unwrap();

    // Wait until both see the full peer list (2 peers)
    let peers_a = wait_for_peer_list(&rv_a, 2).await;
    let _peers_b = wait_for_peer_list(&rv_b, 2).await;

    // Create PeerManagers
    let mgr_a = Arc::new(PeerManager::new(
        endpoint_a,
        peer_cfg_a,
        rv_a.peer_id,
        "alice".to_string(),
    ));
    let mgr_b = Arc::new(PeerManager::new(
        endpoint_b,
        peer_cfg_b,
        rv_b.peer_id,
        "bob".to_string(),
    ));

    // Start accept loops
    mgr_a.spawn_accept_loop();
    mgr_b.spawn_accept_loop();

    // Feed peer lists — the higher peer_id side will initiate the connection
    mgr_a.update_peer_list(peers_a.clone()).await;
    mgr_b.update_peer_list(peers_a).await;

    // Both peers should get PeerConnected events
    let event_a = tokio::time::timeout(Duration::from_secs(5), mgr_a.recv())
        .await
        .expect("timeout waiting for peer A event")
        .unwrap();
    let event_b = tokio::time::timeout(Duration::from_secs(5), mgr_b.recv())
        .await
        .expect("timeout waiting for peer B event")
        .unwrap();

    // Verify both got PeerConnected
    match &event_a {
        NetworkEvent::PeerConnected { peer_id, username } => {
            assert_eq!(*peer_id, rv_b.peer_id);
            assert_eq!(username, "bob");
        }
        other => panic!("expected PeerConnected from A, got {other:?}"),
    }

    match &event_b {
        NetworkEvent::PeerConnected { peer_id, username } => {
            assert_eq!(*peer_id, rv_a.peer_id);
            assert_eq!(username, "alice");
        }
        other => panic!("expected PeerConnected from B, got {other:?}"),
    }

    // Verify connected_peers lists
    assert_eq!(mgr_a.connected_peers().await, vec![rv_b.peer_id]);
    assert_eq!(mgr_b.connected_peers().await, vec![rv_a.peer_id]);
}

#[tokio::test]
async fn peer_message_exchange() {
    use dessplay_core::protocol::CrdtOp;
    use dessplay_core::protocol::LwwValue;
    use dessplay_core::types::{UserId, UserState};

    // Start rendezvous server
    let (server_addr, _server) = start_test_server("pw").await;

    // Create dual endpoints
    let (endpoint_a, peer_cfg_a) = create_test_dual_endpoint();
    let (endpoint_b, peer_cfg_b) = create_test_dual_endpoint();

    // Connect both to rendezvous
    let rv_a = RendezvousClient::connect(&endpoint_a, server_addr, "localhost", "pw", "alice")
        .await
        .unwrap();
    let rv_b = RendezvousClient::connect(&endpoint_b, server_addr, "localhost", "pw", "bob")
        .await
        .unwrap();

    let peers = wait_for_peer_list(&rv_a, 2).await;
    let _peers_b = wait_for_peer_list(&rv_b, 2).await;

    // Create and start PeerManagers
    let mgr_a = Arc::new(PeerManager::new(
        endpoint_a,
        peer_cfg_a,
        rv_a.peer_id,
        "alice".to_string(),
    ));
    let mgr_b = Arc::new(PeerManager::new(
        endpoint_b,
        peer_cfg_b,
        rv_b.peer_id,
        "bob".to_string(),
    ));
    mgr_a.spawn_accept_loop();
    mgr_b.spawn_accept_loop();

    // Feed peer lists
    mgr_a.update_peer_list(peers.clone()).await;
    mgr_b.update_peer_list(peers).await;

    // Wait for connections to establish
    let _ = tokio::time::timeout(Duration::from_secs(5), mgr_a.recv())
        .await
        .expect("timeout A connect")
        .unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), mgr_b.recv())
        .await
        .expect("timeout B connect")
        .unwrap();

    // Send a StateOp from A → B
    let op = CrdtOp::LwwWrite {
        timestamp: 42,
        value: LwwValue::UserState(UserId("alice".into()), UserState::Ready),
    };
    mgr_a
        .send_control(
            rv_b.peer_id,
            &PeerControl::StateOp { op: op.clone() },
        )
        .await
        .unwrap();

    // B should receive it
    let event_b = tokio::time::timeout(Duration::from_secs(5), mgr_b.recv())
        .await
        .expect("timeout waiting for control message on B")
        .unwrap();

    match event_b {
        NetworkEvent::PeerControl { from, message } => {
            assert_eq!(from, rv_a.peer_id);
            match message {
                PeerControl::StateOp { op: received_op } => {
                    assert_eq!(received_op, op);
                }
                other => panic!("expected StateOp, got {other:?}"),
            }
        }
        other => panic!("expected PeerControl, got {other:?}"),
    }
}
