use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use quinn::crypto::rustls::QuicServerConfig;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use tokio::sync::RwLock;

use dessplay::network::rendezvous::{
    ClientMessage, PeerEntry, ServerMessage, cert_fingerprint, decode_relay_header,
    encode_relay_header, read_message, write_message,
};

#[derive(Parser)]
#[command(name = "dessplay-rendezvous", about = "DessPlay rendezvous server")]
struct Args {
    /// Address to bind to
    #[arg(long, default_value = "[::]:4433")]
    bind: SocketAddr,

    /// Password (alternative to DESSPLAY_PASSWORD env var or --password-file)
    #[arg(long)]
    password: Option<String>,

    /// Path to password file (alternative to DESSPLAY_PASSWORD env var)
    #[arg(long, conflicts_with = "password")]
    password_file: Option<PathBuf>,

    /// Directory for persistent data (cert, key)
    #[arg(long, default_value = ".")]
    data_dir: PathBuf,
}

struct ClientState {
    peer_id: String,
    connection: quinn::Connection,
    addrs: Vec<SocketAddr>,
}

type Registry = Arc<RwLock<HashMap<String, ClientState>>>;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = Args::parse();

    let password = if let Some(pw) = args.password {
        pw
    } else if let Some(ref path) = args.password_file {
        std::fs::read_to_string(path)?.trim().to_string()
    } else if let Ok(pw) = std::env::var("DESSPLAY_PASSWORD") {
        pw
    } else {
        anyhow::bail!(
            "password required: use --password, --password-file, or DESSPLAY_PASSWORD env var"
        );
    };

    let (server_config, fingerprint) = make_server_config(&args.data_dir)?;

    tracing::info!(bind = %args.bind, "starting rendezvous server");
    tracing::info!("server fingerprint: {fingerprint}");
    // Also print to stdout for easy copy-paste
    println!("Server fingerprint: {fingerprint}");

    let endpoint = quinn::Endpoint::server(server_config, args.bind)?;
    let registry: Registry = Arc::new(RwLock::new(HashMap::new()));

    while let Some(incoming) = endpoint.accept().await {
        let registry = Arc::clone(&registry);
        let password = password.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(incoming, &password, &registry).await {
                tracing::warn!("connection handler error: {e}");
            }
        });
    }

    Ok(())
}

async fn handle_connection(
    incoming: quinn::Incoming,
    password: &str,
    registry: &Registry,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let connection = incoming.await?;
    let remote_addr = connection.remote_address();
    tracing::info!(addr = %remote_addr, "new connection");

    // Accept control stream
    let (mut send, mut recv) = connection.accept_bi().await?;

    // Read Register message
    let msg: ClientMessage = read_message(&mut recv).await.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { format!("{e}").into() })?;

    let peer_id = match msg {
        ClientMessage::Register {
            peer_id,
            password: pw,
        } => {
            if pw != password {
                tracing::warn!(peer = %peer_id, addr = %remote_addr, "auth failed");
                let resp = ServerMessage::AuthFailed {
                    reason: "invalid password".to_string(),
                };
                write_message(&mut send, &resp).await.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { format!("{e}").into() })?;
                let _ = send.finish();
                // Close with reason — delivered reliably via QUIC CONNECTION_CLOSE
                connection.close(1u32.into(), b"auth failed: invalid password");
                return Ok(());
            }
            peer_id
        }
        ClientMessage::Keepalive => {
            tracing::warn!(addr = %remote_addr, "got keepalive before register");
            return Ok(());
        }
    };

    tracing::info!(peer = %peer_id, addr = %remote_addr, "peer registered");

    // Build initial peer list (everyone except this peer)
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

    // Send Registered response
    let resp = ServerMessage::Registered {
        peers,
        your_addr: remote_addr,
    };
    write_message(&mut send, &resp).await.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { format!("{e}").into() })?;

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

    // Spawn relay tasks
    let registry_clone = Arc::clone(registry);
    let peer_id_clone = peer_id.clone();
    let conn_clone = connection.clone();
    let datagram_relay_task = tokio::spawn(async move {
        relay_datagrams(conn_clone, &peer_id_clone, &registry_clone).await;
    });

    let registry_clone = Arc::clone(registry);
    let peer_id_clone = peer_id.clone();
    let conn_clone = connection.clone();
    let stream_relay_task = tokio::spawn(async move {
        relay_streams(conn_clone, &peer_id_clone, &registry_clone).await;
    });

    // Control loop: process keepalives
    let result = control_loop(&mut send, &mut recv, &peer_id, registry).await;

    // Cleanup
    datagram_relay_task.abort();
    stream_relay_task.abort();

    {
        let mut reg = registry.write().await;
        reg.remove(&peer_id);
    }
    tracing::info!(peer = %peer_id, "peer disconnected");

    result.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { format!("{e}").into() })
}

async fn control_loop(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    peer_id: &str,
    registry: &Registry,
) -> Result<(), dessplay::network::ConnectionError> {
    loop {
        let msg: ClientMessage = match read_message(recv).await {
            Ok(m) => m,
            Err(_) => return Ok(()), // client disconnected
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
                write_message(send, &ServerMessage::PeerList { peers }).await?;
            }
            ClientMessage::Register { .. } => {
                tracing::warn!(peer = %peer_id, "duplicate register ignored");
            }
        }
    }
}

async fn relay_datagrams(connection: quinn::Connection, source_peer: &str, registry: &Registry) {
    loop {
        let datagram = match connection.read_datagram().await {
            Ok(d) => d,
            Err(_) => return, // connection closed
        };

        let Some((dest_peer_id, payload)) = decode_relay_header(&datagram) else {
            tracing::warn!(peer = %source_peer, "invalid relay datagram header");
            continue;
        };

        // Look up destination and forward with source header
        let reg = registry.read().await;
        if let Some(dest) = reg.get(&dest_peer_id) {
            let forwarded = encode_relay_header(source_peer, payload);
            if let Err(e) = dest.connection.send_datagram(forwarded.into()) {
                tracing::debug!(
                    src = %source_peer, dest = %dest_peer_id,
                    "relay datagram failed: {e}"
                );
            }
        } else {
            tracing::debug!(
                src = %source_peer, dest = %dest_peer_id,
                "relay: destination peer not found"
            );
        }
    }
}

async fn relay_streams(connection: quinn::Connection, source_peer: &str, registry: &Registry) {
    loop {
        let mut recv = match connection.accept_uni().await {
            Ok(r) => r,
            Err(_) => return, // connection closed
        };

        let source_peer = source_peer.to_string();
        let registry = Arc::clone(registry);
        tokio::spawn(async move {
            // Read the entire stream (length-limited)
            let data = match recv.read_to_end(16 * 1024 * 1024).await {
                Ok(d) => d,
                Err(e) => {
                    tracing::debug!(peer = %source_peer, "relay stream read error: {e}");
                    return;
                }
            };

            let Some((dest_peer_id, payload)) = decode_relay_header(&data) else {
                tracing::warn!(peer = %source_peer, "invalid relay stream header");
                return;
            };

            let reg = registry.read().await;
            if let Some(dest) = reg.get(&dest_peer_id) {
                let forwarded = encode_relay_header(&source_peer, payload);
                match dest.connection.open_uni().await {
                    Ok(mut send) => {
                        if let Err(e) = send.write_all(&forwarded).await {
                            tracing::debug!(
                                src = %source_peer, dest = %dest_peer_id,
                                "relay stream write error: {e}"
                            );
                        }
                        let _ = send.finish();
                    }
                    Err(e) => {
                        tracing::debug!(
                            src = %source_peer, dest = %dest_peer_id,
                            "relay stream open error: {e}"
                        );
                    }
                }
            }
        });
    }
}

fn make_server_config(
    data_dir: &PathBuf,
) -> anyhow::Result<(quinn::ServerConfig, String)> {
    let cert_path = data_dir.join("cert.der");
    let key_path = data_dir.join("key.der");

    let (cert_der, key_der) = if cert_path.exists() && key_path.exists() {
        tracing::info!("loading existing certificate from {}", data_dir.display());
        let cert = std::fs::read(&cert_path)?;
        let key = std::fs::read(&key_path)?;
        (cert, key)
    } else {
        tracing::info!(
            "generating new self-signed certificate in {}",
            data_dir.display()
        );
        let certified_key =
            rcgen::generate_simple_self_signed(vec!["dessplay-rendezvous".into()])?;
        let cert = certified_key.cert.der().to_vec();
        let key = certified_key.key_pair.serialize_der();
        std::fs::create_dir_all(data_dir)?;
        std::fs::write(&cert_path, &cert)?;
        std::fs::write(&key_path, &key)?;
        (cert, key)
    };

    let fingerprint = cert_fingerprint(&cert_der);
    let cert = CertificateDer::from(cert_der);
    let key = PrivatePkcs8KeyDer::from(key_der);

    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(std::time::Duration::from_secs(5)));
    transport.max_idle_timeout(Some(
        quinn::IdleTimeout::try_from(std::time::Duration::from_secs(30)).unwrap(),
    ));

    let crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key.into())?;

    let mut server_config = quinn::ServerConfig::with_crypto(Arc::new(
        QuicServerConfig::try_from(crypto)?,
    ));
    server_config.transport = Arc::new(transport);

    Ok((server_config, fingerprint))
}
