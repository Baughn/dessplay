use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dessplay::network::quic::QuicConnectionManager;
use dessplay::network::rendezvous::{
    ClientMessage, PeerEntry, RendezvousClient, ServerMessage, cert_fingerprint,
    decode_relay_header, encode_relay_header, read_message, write_message,
};
use dessplay::network::{ConnectionManager, PeerId};

// --- Server helpers ---

/// Minimal in-process rendezvous server for testing.
struct TestServer {
    endpoint: quinn::Endpoint,
    #[allow(dead_code)]
    password: String,
    #[allow(dead_code)]
    cert_der: Vec<u8>,
}

impl TestServer {
    async fn start(password: &str) -> Self {
        let (server_config, cert_der) = make_test_server_config();
        let endpoint = quinn::Endpoint::server(server_config, "[::1]:0".parse().unwrap()).unwrap();

        let ep = endpoint.clone();
        let pw = password.to_string();
        tokio::spawn(async move {
            server_accept_loop(ep, pw).await;
        });

        Self {
            endpoint,
            password: password.to_string(),
            cert_der,
        }
    }

    fn addr(&self) -> SocketAddr {
        self.endpoint.local_addr().unwrap()
    }

    #[allow(dead_code)]
    fn fingerprint(&self) -> String {
        cert_fingerprint(&self.cert_der)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.endpoint.close(0u32.into(), b"shutdown");
    }
}

async fn server_accept_loop(endpoint: quinn::Endpoint, password: String) {
    use std::collections::HashMap;
    use tokio::sync::RwLock;

    struct ClientState {
        peer_id: String,
        connection: quinn::Connection,
        addrs: Vec<SocketAddr>,
    }

    let registry: Arc<RwLock<HashMap<String, ClientState>>> =
        Arc::new(RwLock::new(HashMap::new()));

    while let Some(incoming) = endpoint.accept().await {
        let password = password.clone();
        let registry = Arc::clone(&registry);
        tokio::spawn(async move {
            let connection = match incoming.await {
                Ok(c) => c,
                Err(_) => return,
            };
            let remote_addr = connection.remote_address();

            let (mut send, mut recv) = match connection.accept_bi().await {
                Ok(s) => s,
                Err(_) => return,
            };

            let msg: ClientMessage = match read_message(&mut recv).await {
                Ok(m) => m,
                Err(_) => return,
            };

            let peer_id = match msg {
                ClientMessage::Register {
                    peer_id,
                    password: pw,
                } => {
                    if pw != password {
                        let _ = write_message(
                            &mut send,
                            &ServerMessage::AuthFailed {
                                reason: "invalid password".into(),
                            },
                        )
                        .await;
                        return;
                    }
                    peer_id
                }
                _ => return,
            };

            // Build peer list
            let peers = {
                let reg = registry.read().await;
                reg.values()
                    .filter(|c| c.peer_id != peer_id)
                    .map(|c| PeerEntry {
                        peer_id: c.peer_id.clone(),
                        addrs: c.addrs.clone(),
                    })
                    .collect::<Vec<_>>()
            };

            let _ = write_message(
                &mut send,
                &ServerMessage::Registered {
                    peers,
                    your_addr: remote_addr,
                },
            )
            .await;

            // Add to registry
            {
                let mut reg = registry.write().await;
                reg.insert(
                    peer_id.clone(),
                    ClientState {
                        peer_id: peer_id.clone(),
                        connection: connection.clone(),
                        addrs: vec![remote_addr],
                    },
                );
            }

            // Spawn relay datagram forwarder
            let registry_dg = Arc::clone(&registry);
            let conn_dg = connection.clone();
            let peer_id_dg = peer_id.clone();
            tokio::spawn(async move {
                loop {
                    match conn_dg.read_datagram().await {
                        Ok(data) => {
                            if let Some((dest, payload)) = decode_relay_header(&data) {
                                let reg = registry_dg.read().await;
                                if let Some(dest_state) = reg.get(&dest) {
                                    let forwarded = encode_relay_header(&peer_id_dg, payload);
                                    let _ = dest_state
                                        .connection
                                        .send_datagram(forwarded.into());
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
            });

            // Spawn relay stream forwarder
            let registry_st = Arc::clone(&registry);
            let conn_st = connection.clone();
            let peer_id_st = peer_id.clone();
            tokio::spawn(async move {
                loop {
                    match conn_st.accept_uni().await {
                        Ok(mut recv) => {
                            let registry_st = Arc::clone(&registry_st);
                            let peer_id_st = peer_id_st.clone();
                            tokio::spawn(async move {
                                let data = match recv.read_to_end(16 * 1024 * 1024).await {
                                    Ok(d) => d,
                                    Err(_) => return,
                                };
                                if let Some((dest, payload)) = decode_relay_header(&data) {
                                    let reg = registry_st.read().await;
                                    if let Some(dest_state) = reg.get(&dest) {
                                        let forwarded = encode_relay_header(&peer_id_st, payload);
                                        if let Ok(mut send) =
                                            dest_state.connection.open_uni().await
                                        {
                                            let _ = send.write_all(&forwarded).await;
                                            let _ = send.finish();
                                        }
                                    }
                                }
                            });
                        }
                        Err(_) => break,
                    }
                }
            });

            // Control loop
            loop {
                let msg: ClientMessage = match read_message(&mut recv).await {
                    Ok(m) => m,
                    Err(_) => break,
                };
                match msg {
                    ClientMessage::Keepalive => {
                        let peers = {
                            let reg = registry.read().await;
                            reg.values()
                                .filter(|c| c.peer_id != peer_id)
                                .map(|c| PeerEntry {
                                    peer_id: c.peer_id.clone(),
                                    addrs: c.addrs.clone(),
                                })
                                .collect::<Vec<_>>()
                        };
                        if write_message(&mut send, &ServerMessage::PeerList { peers })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    _ => {}
                }
            }

            // Cleanup
            let mut reg = registry.write().await;
            reg.remove(&peer_id);
        });
    }
}

fn make_test_server_config() -> (quinn::ServerConfig, Vec<u8>) {
    use quinn::crypto::rustls::QuicServerConfig;
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

    let certified_key =
        rcgen::generate_simple_self_signed(vec!["dessplay-rendezvous".into()]).unwrap();
    let cert_der = certified_key.cert.der().to_vec();
    let priv_key = PrivatePkcs8KeyDer::from(certified_key.key_pair.serialize_der());
    let cert = CertificateDer::from(cert_der.clone());

    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(Duration::from_secs(5)));
    transport.max_idle_timeout(Some(
        quinn::IdleTimeout::try_from(Duration::from_secs(30)).unwrap(),
    ));

    let crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], priv_key.into())
        .unwrap();

    let mut server_config =
        quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(crypto).unwrap()));
    server_config.transport = Arc::new(transport);

    (server_config, cert_der)
}

/// Create a temporary directory for TOFU known_servers file.
fn temp_known_servers() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("known_servers");
    (dir, path)
}

/// Create a QuicConnectionManager and get its endpoint for rendezvous use.
async fn make_client(name: &str) -> QuicConnectionManager {
    QuicConnectionManager::new("[::1]:0".parse().unwrap(), PeerId(name.into()))
        .await
        .unwrap()
}

// --- Tests ---

#[tokio::test]
async fn server_starts_and_client_authenticates() {
    let server = TestServer::start("secret123").await;
    let client = make_client("alice").await;
    let (_dir, known_servers) = temp_known_servers();

    let (mut rendezvous, peers, your_addr) = RendezvousClient::connect(
        client.endpoint(),
        server.addr(),
        "alice",
        "secret123",
        &known_servers,
    )
    .await
    .unwrap();

    // No other peers yet
    assert!(peers.is_empty());
    // Should have received our observed address
    assert_ne!(your_addr.port(), 0);

    // Keepalive should work
    let peers = rendezvous.keepalive().await.unwrap();
    assert!(peers.is_empty());
}

#[tokio::test]
async fn auth_failure() {
    let server = TestServer::start("correct-password").await;
    let client = make_client("alice").await;
    let (_dir, known_servers) = temp_known_servers();

    let result = RendezvousClient::connect(
        client.endpoint(),
        server.addr(),
        "alice",
        "wrong-password",
        &known_servers,
    )
    .await;

    match result {
        Err(e) => {
            let msg = e.to_string();
            // Server sends AuthFailed then the handler returns, which may close
            // the connection before the client reads the response.
            assert!(
                msg.contains("auth failed") || msg.contains("connection") || msg.contains("reset"),
                "unexpected error: {msg}"
            );
        }
        Ok(_) => panic!("expected auth failure"),
    }
}

#[tokio::test]
async fn peer_discovery_on_register() {
    let server = TestServer::start("pass").await;
    let client_a = make_client("alice").await;
    let client_b = make_client("bob").await;
    let (_dir_a, ks_a) = temp_known_servers();
    let (_dir_b, ks_b) = temp_known_servers();

    // Alice connects first
    let (_rendezvous_a, peers_a, _) = RendezvousClient::connect(
        client_a.endpoint(),
        server.addr(),
        "alice",
        "pass",
        &ks_a,
    )
    .await
    .unwrap();
    assert!(peers_a.is_empty());

    // Bob connects — should see Alice
    let (_rendezvous_b, peers_b, _) = RendezvousClient::connect(
        client_b.endpoint(),
        server.addr(),
        "bob",
        "pass",
        &ks_b,
    )
    .await
    .unwrap();
    assert_eq!(peers_b.len(), 1);
    assert_eq!(peers_b[0].peer_id, "alice");

    // Keep rendezvous clients alive
    drop(_rendezvous_a);
    drop(_rendezvous_b);
}

#[tokio::test]
async fn keepalive_returns_new_peers() {
    let server = TestServer::start("pass").await;
    let client_a = make_client("alice").await;
    let client_b = make_client("bob").await;
    let (_dir_a, ks_a) = temp_known_servers();
    let (_dir_b, ks_b) = temp_known_servers();

    // Alice connects
    let (mut rendezvous_a, _, _) = RendezvousClient::connect(
        client_a.endpoint(),
        server.addr(),
        "alice",
        "pass",
        &ks_a,
    )
    .await
    .unwrap();

    // Bob connects
    let (_rendezvous_b, _, _) = RendezvousClient::connect(
        client_b.endpoint(),
        server.addr(),
        "bob",
        "pass",
        &ks_b,
    )
    .await
    .unwrap();

    // Alice's keepalive should now see Bob
    let peers = rendezvous_a.keepalive().await.unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].peer_id, "bob");
}

#[tokio::test]
async fn mesh_bootstrap_via_rendezvous() {
    let server = TestServer::start("pass").await;
    let a = make_client("a").await;
    let b = make_client("b").await;
    let c = make_client("c").await;
    let (_dir_a, ks_a) = temp_known_servers();
    let (_dir_b, ks_b) = temp_known_servers();
    let (_dir_c, ks_c) = temp_known_servers();

    // All three register
    let (_ra, _, _) = RendezvousClient::connect(
        a.endpoint(),
        server.addr(),
        "a",
        "pass",
        &ks_a,
    )
    .await
    .unwrap();

    let (_rb, peers_b, _) = RendezvousClient::connect(
        b.endpoint(),
        server.addr(),
        "b",
        "pass",
        &ks_b,
    )
    .await
    .unwrap();
    assert_eq!(peers_b.len(), 1); // sees a

    let (_rc, peers_c, _) = RendezvousClient::connect(
        c.endpoint(),
        server.addr(),
        "c",
        "pass",
        &ks_c,
    )
    .await
    .unwrap();
    assert_eq!(peers_c.len(), 2); // sees a and b

    // Now form direct connections using discovered addresses
    // b connects to a
    let a_addr = peers_b.iter().find(|p| p.peer_id == "a").unwrap().addrs[0];
    b.connect_to(a_addr).await.unwrap();

    // c connects to a and b
    for peer in &peers_c {
        c.connect_to(peer.addrs[0]).await.unwrap();
    }

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify mesh: a has b and c as peers
    let a_peers = a.connected_peers();
    assert_eq!(a_peers.len(), 2);

    // Exchange a datagram a → b
    a.send_datagram(&PeerId("b".into()), b"hello from a")
        .await
        .unwrap();
    let (from, data) = b.recv_datagram().await.unwrap();
    assert_eq!(from, PeerId("a".into()));
    assert_eq!(data, b"hello from a");
}

#[tokio::test]
async fn relay_datagram() {
    let server = TestServer::start("pass").await;
    let a = make_client("alice").await;
    let b = make_client("bob").await;
    let (_dir_a, ks_a) = temp_known_servers();
    let (_dir_b, ks_b) = temp_known_servers();

    // Both register
    let (ra, _, _) = RendezvousClient::connect(
        a.endpoint(),
        server.addr(),
        "alice",
        "pass",
        &ks_a,
    )
    .await
    .unwrap();

    let (rb, _, _) = RendezvousClient::connect(
        b.endpoint(),
        server.addr(),
        "bob",
        "pass",
        &ks_b,
    )
    .await
    .unwrap();

    // Set up relay connections (instead of direct)
    a.set_relay(ra.connection().clone());
    b.set_relay(rb.connection().clone());
    a.add_relayed_peer(PeerId("bob".into()));
    b.add_relayed_peer(PeerId("alice".into()));

    // Send datagram via relay: alice → bob
    a.send_datagram(&PeerId("bob".into()), b"relayed hello")
        .await
        .unwrap();

    let (from, data) = tokio::time::timeout(Duration::from_secs(5), b.recv_datagram())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(from, PeerId("alice".into()));
    assert_eq!(data, b"relayed hello");
}

#[tokio::test]
async fn relay_reliable() {
    let server = TestServer::start("pass").await;
    let a = make_client("alice").await;
    let b = make_client("bob").await;
    let (_dir_a, ks_a) = temp_known_servers();
    let (_dir_b, ks_b) = temp_known_servers();

    let (ra, _, _) = RendezvousClient::connect(
        a.endpoint(),
        server.addr(),
        "alice",
        "pass",
        &ks_a,
    )
    .await
    .unwrap();

    let (rb, _, _) = RendezvousClient::connect(
        b.endpoint(),
        server.addr(),
        "bob",
        "pass",
        &ks_b,
    )
    .await
    .unwrap();

    a.set_relay(ra.connection().clone());
    b.set_relay(rb.connection().clone());
    a.add_relayed_peer(PeerId("bob".into()));
    b.add_relayed_peer(PeerId("alice".into()));

    // Send reliable message via relay
    a.send_reliable(&PeerId("bob".into()), b"reliable relayed")
        .await
        .unwrap();

    let (from, data) = tokio::time::timeout(Duration::from_secs(5), b.recv_reliable())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(from, PeerId("alice".into()));
    assert_eq!(data, b"reliable relayed");
}

#[tokio::test]
async fn client_disconnect_removes_from_peer_list() {
    let server = TestServer::start("pass").await;
    let client_a = make_client("alice").await;
    let client_b = make_client("bob").await;
    let (_dir_a, ks_a) = temp_known_servers();
    let (_dir_b, ks_b) = temp_known_servers();

    // Alice and Bob both register
    let (mut rendezvous_a, _, _) = RendezvousClient::connect(
        client_a.endpoint(),
        server.addr(),
        "alice",
        "pass",
        &ks_a,
    )
    .await
    .unwrap();

    let (rendezvous_b, _, _) = RendezvousClient::connect(
        client_b.endpoint(),
        server.addr(),
        "bob",
        "pass",
        &ks_b,
    )
    .await
    .unwrap();

    // Alice sees Bob
    let peers = rendezvous_a.keepalive().await.unwrap();
    assert_eq!(peers.len(), 1);

    // Bob disconnects
    drop(rendezvous_b);
    client_b.close();

    // Give time for server to detect disconnect
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Alice's keepalive should show empty list
    let peers = rendezvous_a.keepalive().await.unwrap();
    assert!(peers.is_empty(), "expected empty, got {peers:?}");
}

#[tokio::test]
async fn observed_address_stun() {
    let server = TestServer::start("pass").await;
    let client = make_client("alice").await;
    let (_dir, known_servers) = temp_known_servers();

    let (_, _, your_addr) = RendezvousClient::connect(
        client.endpoint(),
        server.addr(),
        "alice",
        "pass",
        &known_servers,
    )
    .await
    .unwrap();

    // The observed address should be a valid loopback address with a port
    assert!(your_addr.ip().is_loopback());
    assert_ne!(your_addr.port(), 0);
}

#[tokio::test]
async fn tofu_accept_on_first_connect() {
    let server = TestServer::start("pass").await;
    let client = make_client("alice").await;
    let (_dir, known_servers) = temp_known_servers();

    // First connect — should succeed and store fingerprint
    let (_, _, _) = RendezvousClient::connect(
        client.endpoint(),
        server.addr(),
        "alice",
        "pass",
        &known_servers,
    )
    .await
    .unwrap();

    // Verify fingerprint was stored
    let contents = std::fs::read_to_string(&known_servers).unwrap();
    assert!(contents.contains("SHA256:"));
    assert!(contents.contains("dessplay-rendezvous"));

    // Second connect to same server — should also succeed
    let client2 = make_client("alice2").await;
    let result = RendezvousClient::connect(
        client2.endpoint(),
        server.addr(),
        "alice2",
        "pass",
        &known_servers,
    )
    .await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn tofu_reject_on_cert_change() {
    let server1 = TestServer::start("pass").await;
    let server1_addr = server1.addr();
    let client = make_client("alice").await;
    let (_dir, known_servers) = temp_known_servers();

    // First connect — stores fingerprint
    let (_, _, _) = RendezvousClient::connect(
        client.endpoint(),
        server1_addr,
        "alice",
        "pass",
        &known_servers,
    )
    .await
    .unwrap();

    // Stop first server
    drop(server1);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Start a NEW server on a different port (different cert)
    let server2 = TestServer::start("pass").await;

    // Manually write a fingerprint for the new server's address
    // using the OLD server's fingerprint format, to simulate cert change.
    // Instead, we write a fake fingerprint for dessplay-rendezvous
    // to the known_servers file to force a mismatch.
    let contents = std::fs::read_to_string(&known_servers).unwrap();
    // The stored entry is for "dessplay-rendezvous" - both servers use same server name
    assert!(contents.contains("dessplay-rendezvous"));

    // Connect to server2 — the TOFU verifier checks the server name "dessplay-rendezvous"
    // which already has a stored fingerprint from server1. Server2 has a different cert.
    let client2 = make_client("alice2").await;
    let result = RendezvousClient::connect(
        client2.endpoint(),
        server2.addr(),
        "alice2",
        "pass",
        &known_servers,
    )
    .await;

    match result {
        Err(e) => {
            let err = e.to_string();
            assert!(
                err.contains("certificate has changed")
                    || err.contains("invalid peer certificate")
                    || err.contains("cert"),
                "unexpected error: {err}"
            );
        }
        Ok(_) => panic!("expected cert mismatch error"),
    }
}
