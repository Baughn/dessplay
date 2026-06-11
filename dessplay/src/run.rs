//! The headless composition root: everything `main()` does beyond flag
//! parsing. Builds the QUIC connector (TOFU-pinned), loads settings and
//! stored state, spawns the client actors, and runs the event loop
//! until Ctrl-C (graceful Goodbye) or auth failure.
//!
//! Phase 6 grows an interactive sibling with the UI actor; `--seeder`
//! keeps using this one forever. See architecture.md (Composition
//! Root).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use dessplay_core::net::quic::QuicConnector;
use dessplay_core::net::{DEFAULT_PORT, Role};
use dessplay_core::types::UserId;
use tokio::sync::mpsc;

use crate::actors::network::{NetworkCommand, NetworkEvent};
use crate::actors::sync::SyncCommand;
use crate::client::{ClientConfig, ClientEvent, SyncConfigExtras, spawn_client};
use crate::storage::Storage;

/// Everything `main()` parses from flags. `None` means "fall back to
/// stored settings / environment / defaults".
#[derive(Debug, Default)]
pub struct HeadlessArgs {
    /// Run as a seeder: no settings database, flags/env only, never
    /// gates playback.
    pub seeder: bool,
    /// Server `host[:port]`.
    pub server: Option<String>,
    /// Username.
    pub username: Option<String>,
    /// Room password.
    pub password: Option<String>,
    /// Hex-encoded server certificate fingerprint to pin (mainly for
    /// seeders, which persist nothing).
    pub fingerprint: Option<String>,
    /// Settings database override (interactive only).
    pub db_path: Option<PathBuf>,
}

/// The wall clock in unix millis — the client-side equivalent of the
/// server's shared clock source.
fn system_clock() -> Arc<dyn Fn() -> u64 + Send + Sync> {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    })
}

/// Append the default port unless the address already has one. A
/// `host:port` has exactly one colon; a bracketed IPv6 literal carries
/// its port after `]:`; multiple colons without brackets is a bare
/// IPv6 literal that needs wrapping.
fn with_default_port(server: &str) -> String {
    let has_port = match server.matches(':').count() {
        0 => false,
        1 => true,
        _ => server.starts_with('[') && server.contains("]:"),
    };
    if has_port {
        server.to_string()
    } else if server.contains(':') {
        format!("[{server}]:{DEFAULT_PORT}")
    } else {
        format!("{server}:{DEFAULT_PORT}")
    }
}

/// Everything needed to spawn a client against the configured server:
/// the resolved connector, identity, and the settings/TOFU storage
/// handle. Shared by the headless run and the importer.
pub(crate) struct ClientSetup {
    pub connector: Arc<QuicConnector>,
    pub username: String,
    pub password: String,
    /// Settings/TOFU handle (interactive only — seeders persist
    /// nothing).
    pub storage: Option<Storage>,
    pub server_addr_str: String,
    /// No fingerprint was pinned; persist the observed one after the
    /// first successful connect.
    pub first_use: bool,
}

/// Resolve settings (stored < env < flags), the server address, and
/// the TOFU pin.
pub(crate) async fn prepare(args: &HeadlessArgs) -> Result<ClientSetup, String> {
    // Settings: stored for interactive clients (flags override, never
    // persisted), flags/env only for seeders.
    let phase = std::time::Instant::now();
    let storage = if args.seeder {
        None
    } else {
        let path = match &args.db_path {
            Some(path) => path.clone(),
            None => Storage::default_path().ok_or("cannot determine the data directory")?,
        };
        Some(Storage::open(&path).map_err(|e| format!("opening {}: {e}", path.display()))?)
    };
    let settings = match &storage {
        Some(storage) => storage
            .load_settings()
            .map_err(|e| format!("loading settings: {e}"))?,
        None => crate::config::Settings::default(),
    };
    tracing::info!(
        elapsed_ms = phase.elapsed().as_millis() as u64,
        "storage opened and settings loaded"
    );

    let username = args
        .username
        .clone()
        .or(settings.username)
        .ok_or("no username configured; pass --username")?;
    // Precedence: flag > env > stored settings. Log the source (never
    // the password) — an auth rejection is usually a precedence
    // surprise, and the log should settle which password was used.
    let (password, password_source) = match args.password.clone() {
        Some(password) => (Some(password), "flag"),
        None => match std::env::var("DESSPLAY_PASSWORD").ok() {
            Some(password) => (Some(password), "env"),
            None => (settings.password, "stored settings"),
        },
    };
    let password =
        password.ok_or("no password configured; pass --password or set DESSPLAY_PASSWORD")?;
    tracing::info!(source = password_source, "password resolved");
    let server = args.server.clone().unwrap_or(settings.server);

    // Resolve and pin.
    let phase = std::time::Instant::now();
    let server_addr_str = with_default_port(&server);
    let addr: SocketAddr = tokio::net::lookup_host(&server_addr_str)
        .await
        .map_err(|e| format!("resolving {server_addr_str}: {e}"))?
        .next()
        .ok_or_else(|| format!("{server_addr_str} resolved to no addresses"))?;
    tracing::info!(
        resolved = %addr,
        elapsed_ms = phase.elapsed().as_millis() as u64,
        "resolved {server_addr_str}"
    );
    let server_name = server_addr_str
        .rsplit_once(':')
        .map(|(host, _)| host.trim_matches(['[', ']']))
        .unwrap_or(&server_addr_str)
        .to_string();

    let pinned = match &args.fingerprint {
        Some(hex) => Some(decode_hex(hex)?),
        None => match &storage {
            Some(storage) => storage
                .tofu_fingerprint(&server_addr_str)
                .map_err(|e| format!("reading pinned fingerprint: {e}"))?,
            None => None,
        },
    };
    let first_use = pinned.is_none();
    if first_use {
        tracing::warn!("no pinned fingerprint for {server_addr_str}; trusting on first use");
    }
    let phase = std::time::Instant::now();
    let connector = Arc::new(
        QuicConnector::new(addr, server_name, pinned)
            .map_err(|e| format!("building QUIC endpoint: {e}"))?,
    );
    tracing::info!(
        elapsed_ms = phase.elapsed().as_millis() as u64,
        "QUIC endpoint built"
    );

    Ok(ClientSetup {
        connector,
        username,
        password,
        storage,
        server_addr_str,
        first_use,
    })
}

/// Run the headless client until Ctrl-C. Errors are human-readable —
/// `main()` just prints them.
pub async fn run_headless(args: HeadlessArgs) -> Result<(), String> {
    let start = std::time::Instant::now();
    let seeder = args.seeder;
    let db_path = args.db_path.clone();
    let setup = prepare(&args).await?;

    // Stored CRDT state, and a second storage handle for the sync
    // actor (SQLite in WAL mode is fine with two connections; the sync
    // actor owns its handle outright).
    let (initial, sync_storage) = if seeder {
        (None, None)
    } else {
        let path = match db_path {
            Some(path) => path,
            None => Storage::default_path().ok_or("cannot determine the data directory")?,
        };
        let sync_storage =
            Storage::open(&path).map_err(|e| format!("opening {}: {e}", path.display()))?;
        let initial = sync_storage
            .load_state()
            .map_err(|e| format!("loading stored state: {e}"))?;
        (initial, Some(sync_storage))
    };

    let role = if seeder {
        Role::Seeder
    } else {
        Role::Interactive
    };
    let ClientSetup {
        connector,
        username,
        password,
        storage,
        server_addr_str,
        first_use,
    } = setup;
    let mut handle = spawn_client(
        Arc::clone(&connector),
        ClientConfig {
            username: UserId::new(&username),
            password,
            role,
            session_nonce: rand::random(),
            clock: system_clock(),
            sync: SyncConfigExtras {
                initial,
                storage: sync_storage,
                flush_interval: None,
            },
        },
    );
    tracing::info!("{username} ({role:?}) connecting to {server_addr_str}");

    // ---- Event loop until Ctrl-C.
    let mut pin_pending = first_use && !seeder;
    let mut first_connected = true;
    let mut first_peer_list = true;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutting down");
                break;
            }
            event = handle.events.recv() => {
                let Some(event) = event else { break };
                match event {
                    ClientEvent::Network(NetworkEvent::Connected { observed_addr }) => {
                        if first_connected {
                            first_connected = false;
                            tracing::info!(
                                since_start_ms = start.elapsed().as_millis() as u64,
                                "first Connected event"
                            );
                        }
                        tracing::info!("connected (we are {observed_addr})");
                        if pin_pending
                            && let Some(storage) = &storage
                            && let Some(fingerprint) = connector.observed_fingerprint()
                        {
                            let now = (system_clock())() as i64;
                            match storage.store_tofu_fingerprint(
                                &server_addr_str,
                                &fingerprint,
                                now,
                            ) {
                                Ok(()) => {
                                    pin_pending = false;
                                    tracing::info!("pinned server fingerprint on first use");
                                }
                                Err(e) => tracing::error!("storing fingerprint: {e}"),
                            }
                        }
                    }
                    ClientEvent::Network(NetworkEvent::AuthFailed) => {
                        return Err("the server rejected the password".into());
                    }
                    ClientEvent::Network(NetworkEvent::Disconnected { reason }) => {
                        tracing::warn!("disconnected ({reason}); retrying");
                    }
                    ClientEvent::Network(NetworkEvent::PeerList(peers)) => {
                        if first_peer_list {
                            first_peer_list = false;
                            tracing::info!(
                                since_start_ms = start.elapsed().as_millis() as u64,
                                "first PeerList"
                            );
                        }
                        let listed: Vec<String> = peers
                            .iter()
                            .map(|p| format!("{} [{:?}/{:?}]", p.username, p.role, p.presence))
                            .collect();
                        tracing::info!("peers: {}", listed.join(", "));
                    }
                    other => tracing::debug!("{other:?}"),
                }
            }
        }
    }

    // Graceful teardown: Goodbye to the server, flush state to SQLite.
    // Each actor drops its command receiver when it exits, so closed()
    // is the completion signal — bounded, because a wedged actor must
    // never hold the process hostage.
    let _ = handle.network.send(NetworkCommand::Shutdown).await;
    let _ = handle.sync.send(SyncCommand::Shutdown).await;
    let done = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        handle.network.closed().await;
        handle.sync.closed().await;
    })
    .await;
    if done.is_err() {
        tracing::error!("actors did not shut down within 5s; exiting anyway");
    }
    Ok(())
}

/// Run the interactive TUI client until quit.
pub async fn run_interactive(args: HeadlessArgs) -> Result<(), String> {
    use crate::actors::sync::{Mutation, SyncCommand};
    use crate::ui::app::{Ui, UiSnapshot};
    use crate::ui::msg::UserAction;
    use crate::ui::shell::{UiInput, run_input_thread, run_ui_thread};
    use dessplay_core::types::ManualState;

    let start = std::time::Instant::now();
    let db_path = match &args.db_path {
        Some(path) => path.clone(),
        None => Storage::default_path().ok_or("cannot determine the data directory")?,
    };
    // Interactive mode owns first-run setup: whether the settings
    // screen opens is decided by the *stored* settings, before any
    // prefills. Prefills ($USER, the .env password, flags) only become
    // the modal's editable defaults.
    let phase = std::time::Instant::now();
    let mut setup_storage =
        Storage::open(&db_path).map_err(|e| format!("opening {}: {e}", db_path.display()))?;
    let mut settings: crate::config::Settings = setup_storage
        .load_settings()
        .map_err(|e| format!("loading settings: {e}"))?;
    let media_roots = setup_storage
        .media_roots()
        .map_err(|e| format!("loading media roots: {e}"))?;
    tracing::info!(
        elapsed_ms = phase.elapsed().as_millis() as u64,
        "storage opened and settings loaded"
    );
    let needs_setup = settings.needs_setup() || media_roots.is_empty();
    if settings.username.is_none() {
        settings.username = args.username.clone().or_else(|| std::env::var("USER").ok());
    }
    // Track (and log) where the settings password came from — never
    // the password itself.
    let password_source = if settings.password.is_some() {
        "stored settings"
    } else if args.password.is_some() {
        "flag"
    } else if std::env::var_os("DESSPLAY_PASSWORD").is_some() {
        "env"
    } else {
        "unset (settings screen)"
    };
    tracing::info!(source = password_source, "settings password source");
    if settings.password.is_none() {
        settings.password = args
            .password
            .clone()
            .or_else(|| std::env::var("DESSPLAY_PASSWORD").ok());
    }

    // The UI runs on its own threads; this task bridges it to the
    // actors.
    let ui = Ui::with_setup(
        UserId::new(settings.username.clone().unwrap_or_default()),
        settings.clone(),
        media_roots,
        needs_setup,
    );
    let (input_tx, input_rx) = std::sync::mpsc::sync_channel::<UiInput>(64);
    let (action_tx, mut action_rx) = mpsc::channel::<UserAction>(64);
    let ui_thread = std::thread::spawn(move || run_ui_thread(ui, input_rx, action_tx));
    {
        let input_tx = input_tx.clone();
        std::thread::spawn(move || run_input_thread(input_tx));
    }

    // First run: wait for the settings save before connecting (the Ui
    // updates its own identity from the saved username).
    if needs_setup {
        loop {
            match action_rx.recv().await {
                Some(UserAction::SaveSettings(saved, roots)) => {
                    setup_storage
                        .save_settings(&saved)
                        .map_err(|e| format!("saving settings: {e}"))?;
                    setup_storage
                        .set_media_roots(&roots)
                        .map_err(|e| format!("saving media roots: {e}"))?;
                    settings = *saved;
                    break;
                }
                Some(UserAction::Quit) | None => {
                    let _ = input_tx.try_send(UiInput::Shutdown);
                    let _ = ui_thread.join();
                    return Ok(());
                }
                Some(_) => continue,
            }
        }
    }
    let me = UserId::new(
        settings
            .username
            .clone()
            .ok_or("settings saved without a username")?,
    );

    let setup = prepare(&args).await?;
    let sync_storage =
        Storage::open(&db_path).map_err(|e| format!("opening {}: {e}", db_path.display()))?;
    let initial = sync_storage
        .load_state()
        .map_err(|e| format!("loading stored state: {e}"))?;
    let mut handle = spawn_client(
        Arc::clone(&setup.connector),
        ClientConfig {
            username: UserId::new(&setup.username),
            password: setup.password.clone(),
            role: Role::Interactive,
            session_nonce: rand::random(),
            clock: system_clock(),
            sync: SyncConfigExtras {
                initial,
                storage: Some(sync_storage),
                flush_interval: None,
            },
        },
    );

    // The player side: policy in `session`, mpv behind it. The player
    // process itself only spawns when something first loads.
    let manual_mappings = setup_storage
        .manual_mappings()
        .map_err(|e| format!("loading manual mappings: {e}"))?
        .into_iter()
        .collect();
    let mut shell = crate::session::SessionShell::new(
        me.clone(),
        crate::player::mpv::MpvFactory::new("mpv"),
        system_clock(),
        setup_storage
            .media_roots()
            .map_err(|e| format!("loading media roots: {e}"))?,
        manual_mappings,
        handle.sync.clone(),
        handle.network.clone(),
    );
    // The view the player layer last saw; refreshed with every UI
    // snapshot, used between snapshots by player/matcher events.
    let mut last_view = dessplay_core::StateView::default();

    /// Build a fresh snapshot for the UI.
    async fn snapshot_for(
        handle: &crate::client::ClientHandle,
        storage: &Storage,
    ) -> Option<UiSnapshot> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        handle.sync.send(SyncCommand::GetView(tx)).await.ok()?;
        let view = rx.await.ok()?;
        let recency = storage
            .recent_watched(500)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|record| Some((record.series_id?, record.watched_at as u64)))
            .collect();
        Some(UiSnapshot {
            view,
            peers: handle.peers.borrow().clone(),
            recency,
        })
    }

    let mut pin_pending = setup.first_use;
    let mut startup_state_written = false;
    let mut first_connected = true;
    let mut first_peer_list = true;
    let mut first_snapshot = true;
    loop {
        tokio::select! {
            action = action_rx.recv() => {
                match action {
                    None | Some(UserAction::Quit) => break,
                    Some(UserAction::Mutate(mutation)) => {
                        let _ = handle
                            .sync
                            .send(SyncCommand::Mutate(Box::new(mutation)))
                            .await;
                    }
                    Some(UserAction::HashAndAdd { path, after }) => {
                        let filename = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string());
                        let hash_path = path.clone();
                        let hashed = tokio::task::spawn_blocking(move || {
                            let file = std::fs::File::open(&hash_path)?;
                            dessplay_core::hash::ed2k_hash_reader(file)
                        })
                        .await;
                        match hashed {
                            Ok(Ok(hashed)) => {
                                let _ = handle
                                    .sync
                                    .send(SyncCommand::Mutate(Box::new(
                                        Mutation::AddPlaylistAfter {
                                            anchor: after,
                                            new: dessplay_core::playlist::NewPlaylistEntry {
                                                hash: hashed.root,
                                                added_by: me.clone(),
                                                filename,
                                                size_bytes: hashed.size_bytes,
                                                // Backfilled by the player's
                                                // duration probe on first load.
                                                duration_millis: None,
                                            },
                                        },
                                    )))
                                    .await;
                                // We picked this file: it is its own
                                // verified local copy.
                                let _ = shell.note_local_file(hashed.root, path).await;
                            }
                            Ok(Err(e)) => tracing::error!("hashing failed: {e}"),
                            Err(e) => tracing::error!("hash task died: {e}"),
                        }
                    }
                    Some(UserAction::SaveSettings(saved, roots)) => {
                        if let Err(e) = setup_storage.save_settings(&saved) {
                            tracing::error!("saving settings: {e}");
                        }
                        // set_media_roots needs &mut; reopen briefly.
                        match Storage::open(&db_path) {
                            Ok(mut storage) => {
                                if let Err(e) = storage.set_media_roots(&roots) {
                                    tracing::error!("saving media roots: {e}");
                                }
                            }
                            Err(e) => tracing::error!("opening storage: {e}"),
                        }
                        shell.media_roots = roots;
                        settings = *saved;
                    }
                }
            }
            event = handle.events.recv() => {
                let Some(event) = event else { break };
                match &event {
                    ClientEvent::Network(NetworkEvent::AuthFailed) => {
                        let _ = input_tx.try_send(UiInput::Shutdown);
                        let _ = ui_thread.join();
                        return Err("the server rejected the password".into());
                    }
                    ClientEvent::Network(NetworkEvent::Connected { .. }) => {
                        if first_connected {
                            first_connected = false;
                            tracing::info!(
                                since_start_ms = start.elapsed().as_millis() as u64,
                                "first Connected event"
                            );
                        }
                        if pin_pending && let Some(fp) = setup.connector.observed_fingerprint() {
                            let now = (system_clock())() as i64;
                            if setup_storage
                                .store_tofu_fingerprint(&setup.server_addr_str, &fp, now)
                                .is_ok()
                            {
                                pin_pending = false;
                            }
                        }
                        // "Ready on startup": write our manual override
                        // once per session (clears a stale Paused too).
                        if !startup_state_written {
                            startup_state_written = true;
                            let state = if settings.ready_on_startup {
                                None
                            } else {
                                Some(ManualState::Paused)
                            };
                            let _ = handle
                                .sync
                                .send(SyncCommand::Mutate(Box::new(
                                    Mutation::SetManualOverride { user: me.clone(), state },
                                )))
                                .await;
                        }
                    }
                    ClientEvent::Network(NetworkEvent::PeerList(_)) if first_peer_list => {
                        first_peer_list = false;
                        tracing::info!(
                            since_start_ms = start.elapsed().as_millis() as u64,
                            "first PeerList"
                        );
                    }
                    ClientEvent::Network(NetworkEvent::ClockSync { offset_millis }) => {
                        shell.set_clock_offset(*offset_millis).await;
                    }
                    _ => {}
                }
                // Any event can change what the UI shows — and what the
                // player layer should be doing.
                if let Some(snapshot) = snapshot_for(&handle, &setup_storage).await {
                    if first_snapshot {
                        first_snapshot = false;
                        tracing::info!(
                            since_start_ms = start.elapsed().as_millis() as u64,
                            "first state snapshot pushed to the UI"
                        );
                    }
                    shell.on_state(&snapshot.view, &snapshot.peers).await;
                    last_view = snapshot.view.clone();
                    let _ = input_tx.try_send(UiInput::Snapshot(Box::new(snapshot)));
                }
            }
            output = shell.player_outputs.recv() => {
                let Some(output) = output else { continue };
                for line in shell.on_player_output(output, &last_view).await {
                    let _ = input_tx.try_send(UiInput::Subtitle(line));
                }
            }
            resolution = shell.resolutions.recv() => {
                let Some((file, resolution)) = resolution else { continue };
                let peers = handle.peers.borrow().clone();
                shell.on_resolution(file, resolution, &last_view, &peers).await;
            }
        }
    }

    // Teardown: release the terminal immediately (the user asked to
    // leave), then Goodbye + flush with a bounded wait — a wedged actor
    // must never hold the process hostage.
    let _ = input_tx.try_send(UiInput::Shutdown);
    let _ = ui_thread.join();
    shell.shutdown().await;
    let _ = handle.network.send(NetworkCommand::Shutdown).await;
    let _ = handle.sync.send(SyncCommand::Shutdown).await;
    let done = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        handle.network.closed().await;
        handle.sync.closed().await;
    })
    .await;
    if done.is_err() {
        tracing::error!("actors did not shut down within 5s; exiting anyway");
    }
    Ok(())
}

/// `dessplay --dump`: print settings and the stored state, then exit.
pub fn run_dump(args: &HeadlessArgs) -> Result<(), String> {
    let path = match &args.db_path {
        Some(path) => path.clone(),
        None => Storage::default_path().ok_or("cannot determine the data directory")?,
    };
    let storage = Storage::open(&path).map_err(|e| format!("opening {}: {e}", path.display()))?;
    let settings = storage
        .load_settings()
        .map_err(|e| format!("loading settings: {e}"))?;
    println!("database: {}", path.display());
    println!("settings: {settings:#?}");
    println!(
        "media roots: {:#?}",
        storage.media_roots().map_err(|e| e.to_string())?
    );
    match storage.load_state().map_err(|e| e.to_string())? {
        None => println!("no stored state"),
        Some(snapshot) => {
            println!("epoch: {:?}", snapshot.epoch);
            println!("state: {:#?}", snapshot.state.view());
        }
    }
    Ok(())
}

/// `dessplay import-list`: parse the exported sheets, print the report,
/// and (unless `dry_run`) push the entries through a transient client.
pub async fn run_import(
    args: HeadlessArgs,
    files: Vec<std::path::PathBuf>,
    watchers: String,
    dry_run: bool,
) -> Result<(), String> {
    let map = crate::import::WatcherMap::parse(&watchers)?;
    let mut report = crate::import::ImportReport::default();
    for file in &files {
        let content = std::fs::read_to_string(file)
            .map_err(|e| format!("reading {}: {e}", file.display()))?;
        let label = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| file.display().to_string());
        crate::import::import_sheet(&content, &label, &map, &mut report)?;
    }

    println!("parsed {} entries:", report.entries.len());
    for (status, count) in report.status_counts() {
        println!("  {status}: {count}");
    }
    if !report.warnings.is_empty() {
        println!("\n{} rows need a human:", report.warnings.len());
        for warning in &report.warnings {
            println!("  {warning}");
        }
    }
    if dry_run {
        println!("\ndry run; nothing submitted");
        return Ok(());
    }

    let setup = prepare(&args).await?;
    let handle = spawn_client(
        Arc::clone(&setup.connector),
        ClientConfig {
            username: UserId::new(&setup.username),
            password: setup.password,
            role: Role::Interactive,
            session_nonce: rand::random(),
            clock: system_clock(),
            // Transient client: no stored state in, none persisted out.
            sync: SyncConfigExtras::default(),
        },
    );
    println!(
        "\nconnecting to {} as {}...",
        setup.server_addr_str, setup.username
    );
    let outcome = crate::import::submit(&handle, &report).await?;
    println!("created {}, updated {}", outcome.created, outcome.updated);

    let _ = handle.network.send(NetworkCommand::Shutdown).await;
    let _ = handle.sync.send(SyncCommand::Shutdown).await;
    handle.network.closed().await;
    handle.sync.closed().await;
    Ok(())
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    let hex: String = hex
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':')
        .collect();
    if !hex.len().is_multiple_of(2) {
        return Err("fingerprint hex has odd length".into());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| format!("bad hex: {e}")))
        .collect()
}

/// Parse one `.env` line into `(key, value)`. Tolerates an optional
/// `export ` prefix (users write `.env` files both ways), `#` comments,
/// and blank lines (both yield `None`).
fn parse_dotenv_line(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").unwrap_or(line);
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim().trim_matches('"')))
}

/// Load `./.env` (KEY=VALUE lines; `#` comments) into the environment,
/// without overriding variables that are already set. The project keeps
/// `DESSPLAY_PASSWORD` for the default server there.
pub fn load_dotenv() {
    let Ok(contents) = std::fs::read_to_string(".env") else {
        return;
    };
    for line in contents.lines() {
        if let Some((key, value)) = parse_dotenv_line(line)
            && std::env::var_os(key).is_none()
        {
            // Single-threaded startup: set_var is safe here.
            unsafe { std::env::set_var(key, value) };
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn default_port_handling() {
        assert_eq!(with_default_port("example.com"), "example.com:9876");
        assert_eq!(with_default_port("example.com:443"), "example.com:443");
        assert_eq!(with_default_port("10.0.0.1"), "10.0.0.1:9876");
        assert_eq!(with_default_port("10.0.0.1:7000"), "10.0.0.1:7000");
        assert_eq!(with_default_port("::1"), "[::1]:9876");
        assert_eq!(with_default_port("[::1]:7000"), "[::1]:7000");
    }

    #[test]
    fn dotenv_line_parsing() {
        assert_eq!(parse_dotenv_line("KEY=value"), Some(("KEY", "value")));
        assert_eq!(
            parse_dotenv_line("export KEY=value"),
            Some(("KEY", "value"))
        );
        assert_eq!(
            parse_dotenv_line("  export KEY = \"quoted\"  "),
            Some(("KEY", "quoted"))
        );
        // `export` is only stripped as a prefix keyword, not from keys.
        assert_eq!(
            parse_dotenv_line("exported=value"),
            Some(("exported", "value"))
        );
        assert_eq!(parse_dotenv_line("# comment"), None);
        assert_eq!(parse_dotenv_line(""), None);
        assert_eq!(parse_dotenv_line("not a kv pair"), None);
    }

    #[test]
    fn hex_decoding() {
        assert_eq!(decode_hex("0aff").unwrap(), vec![0x0a, 0xff]);
        assert_eq!(decode_hex("0A:FF").unwrap(), vec![0x0a, 0xff]);
        assert!(decode_hex("abc").is_err());
        assert!(decode_hex("zz").is_err());
    }
}
