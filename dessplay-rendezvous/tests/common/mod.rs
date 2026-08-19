//! The headless multi-client harness (testing-strategy.md): N real
//! clients (network + sync actors) against the real server over the
//! simulated transport, in one paused-time runtime. Phase 6 added UI
//! handles; Phase 7 adds player clients — a full session shell around a
//! MockPlayer, so scenarios can press space "in mpv" on one client and
//! watch everyone's player obey.
//!
//! Player clients touch the real filesystem (tempdir media roots, the
//! blocking-pool matcher), so their timing is not *perfectly*
//! deterministic the way pure sim tests are; the `eventually` budgets
//! absorb that.

#![allow(dead_code)] // each test binary uses a subset

use std::sync::Arc;
use std::time::Duration;

use dessplay::actors::network::{NetworkCommand, NetworkEvent};
use dessplay::actors::sync::{Mutation, SyncCommand};
use dessplay::client::{ClientConfig, ClientEvent, ClientHandle, SyncConfigExtras, spawn_client};
use dessplay::player::PlayerEvent;
use dessplay::player::mock::{MockCommand, MockControl, MockFactory, MockPlayer};
use dessplay::session::SessionShell;
use dessplay_core::net::sim::{EndpointId, SimNetwork};
use dessplay_core::net::{PeerInfo, Role, ServerControl};
use dessplay_core::playlist::NewPlaylistEntry;
use dessplay_core::types::{Ed2kHash, Epoch, UserId};
use dessplay_core::{StateView, derive};
use dessplay_rendezvous::server::{self, ServerConfig};
use tokio::sync::{mpsc, oneshot, watch};

pub const PASSWORD: &str = "hunter2";

/// Route library tracing into test output. The default surfaces only
/// warnings (e.g. a dropped cross-actor send) so a failing run carries
/// its own diagnosis without spamming green ones; RUST_LOG overrides
/// for a full debug chase. Safe to call from every test — later calls
/// are no-ops.
pub fn init_test_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("dessplay=warn,dessplay_rendezvous=warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .try_init();
}

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
    /// The transfer listener's endpoint (the sim's stand-in for the
    /// control-port+1 convention).
    pub transfer_id: EndpointId,
}

impl Harness {
    /// Server with default config (password only).
    pub fn new(seed: u64) -> Self {
        Self::with_config(seed, ServerConfig::new(PASSWORD))
    }

    /// Server with a custom config (compaction schedules, chat_keep).
    pub fn with_config(seed: u64, config: ServerConfig) -> Self {
        Self::with_config_and_storage(seed, config, None)
    }

    /// Server with custom config and storage — needed by anything
    /// exercising the AniDB worker (its queues live in storage).
    pub fn with_config_and_storage(
        seed: u64,
        config: ServerConfig,
        storage: Option<dessplay_rendezvous::storage::ServerStorage>,
    ) -> Self {
        let net = SimNetwork::new(seed);
        let server_id = EndpointId::new("server");
        let transfer_id = EndpointId::new("server-transfer");
        let listener = net.listener(&server_id);
        let transfer_listener = net.listener(&transfer_id);
        tokio::spawn(server::run(
            listener,
            transfer_listener,
            config,
            sim_clock(0),
            storage,
        ));
        Self {
            net,
            server_id,
            transfer_id,
        }
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
        let transfer_connector = Arc::new(
            self.net
                .connector(&EndpointId::new(name), &self.transfer_id),
        );
        spawn_client(
            connector,
            transfer_connector,
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

    /// Spawn a player client: a full client plus the session shell and
    /// a MockPlayer, pumped exactly the way `run_interactive` pumps
    /// them (minus the terminal). The returned control is the "user in
    /// mpv": inject [`PlayerEvent`]s, observe [`MockCommand`]s.
    pub fn player_client(&self, name: &str, nonce: u128) -> PlayerClient {
        let mut handle = self.client(name, nonce);
        let root = tempfile::tempdir().expect("tempdir");
        let cache_dir = tempfile::tempdir().expect("cache tempdir");
        let (player, control) = MockPlayer::auto_pair();
        let mut shell = SessionShell::new(
            UserId::new(name),
            MockFactory::new([player]),
            sim_clock(0),
            dessplay::actors::file::FileConfig {
                storage: dessplay::storage::Storage::open_in_memory().expect("in-memory storage"),
                media_roots: vec![root.path().to_path_buf()],
                retention: dessplay::config::CacheRetention::default(),
                cache_dir: cache_dir.path().to_path_buf(),
                clock: sim_clock(0),
                download: dessplay::download::DownloadConfig::default(),
                upload_limit: None,
                scan_interval: None,
                scan_transfer_quiet: dessplay::actors::file::SCAN_TRANSFER_QUIET_DEFAULT,
                torrent: None,
                nyaa: None,
            },
            true, // auto_download
            handle.sync.clone(),
            handle.network.clone(),
        );
        let sync = handle.sync.clone();
        let network = handle.network.clone();
        let peers = handle.peers.clone();
        let (ui_lines_tx, ui_lines) = mpsc::unbounded_channel();
        let pump_sync = sync.clone();
        let pump_peers = peers.clone();
        tokio::spawn(async move {
            // The run_interactive loop, terminal-free: every client
            // event refreshes the view and re-derives; player outputs
            // and resolutions feed back through the shell.
            let mut last_view = StateView::default();
            loop {
                tokio::select! {
                    event = handle.events.recv() => {
                        let Some(event) = event else { break };
                        // Data streams are owned, not cloneable:
                        // intercept them before the by-reference checks,
                        // exactly as run_interactive does.
                        let event = match event {
                            ClientEvent::Network(NetworkEvent::TransferStream {
                                peer,
                                file,
                                outbound,
                                stream,
                            }) => {
                                shell.on_transfer_stream(peer, file, outbound, stream).await;
                                continue;
                            }
                            event => event,
                        };
                        if let ClientEvent::Network(NetworkEvent::ClockSync { offset_millis }) =
                            &event
                        {
                            shell.set_clock_offset(*offset_millis).await;
                        }
                        if let ClientEvent::Network(NetworkEvent::Peer { from, message }) = &event {
                            shell.on_network_peer(from.clone(), message.clone()).await;
                        }
                        let (tx, rx) = oneshot::channel();
                        if pump_sync.send(SyncCommand::GetView(tx)).await.is_err() {
                            break;
                        }
                        let Ok(view) = rx.await else { break };
                        let peer_list = pump_peers.borrow().clone();
                        let lines = shell.on_state(&view, &peer_list).await;
                        let _ = ui_lines_tx.send(lines);
                        last_view = view;
                    }
                    output = shell.player_outputs.recv() => {
                        let Some(output) = output else { break };
                        let lines = shell.on_player_output(output, &last_view).await;
                        let _ = ui_lines_tx.send(lines);
                    }
                    output = shell.file_outputs.recv() => {
                        let Some(output) = output else { break };
                        let peer_list = pump_peers.borrow().clone();
                        shell.on_file_output(output, &last_view, &peer_list).await;
                    }
                }
            }
        });
        PlayerClient {
            name: name.to_string(),
            sync,
            network,
            peers,
            control,
            ui_lines,
            root,
            _cache_dir: cache_dir,
        }
    }

    /// Spawn a seeder: a headless client (Role::Seeder) plus a
    /// `SeederTransfer` driver, pumped the way `run_headless` pumps it.
    /// It auto-fetches every playlist entry and serves it. `media`
    /// pre-seeds files into its (persistent) store via the cache dir.
    pub fn seeder_client(&self, name: &str, nonce: u128) -> SeederClient {
        use dessplay::seeder::{SeederTransfer, seeder_file_config};
        let handle = self.seeder(name, nonce);
        let media = tempfile::tempdir().expect("media tempdir");
        let cache = tempfile::tempdir().expect("cache tempdir");
        let (mut transfer, mut file_outputs) = SeederTransfer::new(
            UserId::new(name),
            seeder_file_config(
                dessplay::storage::Storage::open_in_memory().expect("storage"),
                vec![media.path().to_path_buf()],
                cache.path().to_path_buf(),
                sim_clock(0),
                None,
                dessplay::download::DownloadConfig::default(),
            ),
            handle.sync.clone(),
            handle.network.clone(),
        );
        let sync = handle.sync.clone();
        let network = handle.network.clone();
        let peers = handle.peers.clone();
        let pump_sync = sync.clone();
        let pump_peers = peers.clone();
        tokio::spawn(async move {
            let mut handle = handle;
            loop {
                tokio::select! {
                    event = handle.events.recv() => {
                        let Some(event) = event else { break };
                        let event = match event {
                            ClientEvent::Network(NetworkEvent::TransferStream {
                                peer,
                                file,
                                outbound,
                                stream,
                            }) => {
                                transfer.on_transfer_stream(peer, file, outbound, stream).await;
                                continue;
                            }
                            event => event,
                        };
                        if let ClientEvent::Network(NetworkEvent::Peer { from, message }) = &event {
                            transfer.on_peer(from.clone(), message.clone()).await;
                        }
                        let (tx, rx) = oneshot::channel();
                        if pump_sync.send(SyncCommand::GetView(tx)).await.is_err() {
                            break;
                        }
                        let Ok(view) = rx.await else { break };
                        let peer_list = pump_peers.borrow().clone();
                        transfer.on_state(&view, &peer_list).await;
                    }
                    output = file_outputs.recv() => {
                        let Some(output) = output else { break };
                        transfer.on_file_output(output).await;
                    }
                }
            }
        });
        SeederClient {
            sync,
            network,
            peers,
            _media: media,
            _cache: cache,
        }
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

/// A seeder: the `SeederTransfer` driver pumps in the background. Holds
/// the channels for snapshots/mutations and keeps its tempdirs alive.
pub struct SeederClient {
    pub sync: mpsc::Sender<SyncCommand>,
    pub network: mpsc::Sender<NetworkCommand>,
    pub peers: watch::Receiver<Vec<PeerInfo>>,
    _media: tempfile::TempDir,
    _cache: tempfile::TempDir,
}

impl SnapshotSource for SeederClient {
    fn sync_tx(&self) -> &mpsc::Sender<SyncCommand> {
        &self.sync
    }
    fn network_tx(&self) -> &mpsc::Sender<NetworkCommand> {
        &self.network
    }
    fn peers_rx(&self) -> &watch::Receiver<Vec<PeerInfo>> {
        &self.peers
    }
}

/// A player-enabled client: the session shell pumps in the background;
/// this is what the test holds.
pub struct PlayerClient {
    pub name: String,
    pub sync: mpsc::Sender<SyncCommand>,
    pub network: mpsc::Sender<NetworkCommand>,
    pub peers: watch::Receiver<Vec<PeerInfo>>,
    /// The "user in mpv": inject events, observe commands.
    pub control: MockControl,
    /// Local-only narrator/subtitle effects produced by the real session shell.
    pub ui_lines: mpsc::UnboundedReceiver<dessplay::session::UiLines>,
    /// This client's media root.
    pub root: tempfile::TempDir,
    /// The file actor's download cache (placeholder PNG home); kept
    /// alive so it isn't deleted out from under the actor.
    _cache_dir: tempfile::TempDir,
}

impl PlayerClient {
    /// Put a media file into this client's root.
    pub fn install(&self, file: &MediaFile) {
        std::fs::write(self.root.path().join(&file.filename), &file.contents)
            .expect("writing media file");
    }

    /// The user does something in their player.
    pub fn user(&self, event: PlayerEvent) {
        self.control.events.send(event).expect("player pump gone");
    }

    /// Wait for a `loadfile`. The auto-acking mock confirms it the way
    /// real mpv does — the observed `path` echo, then file-loaded, then
    /// duration — so by the time the Load command is visible here,
    /// file-attributed events (positions, seeks, EOF) are attributed
    /// (design.md, Events from Player: evidence-based attribution).
    /// Returns the loaded path.
    pub async fn expect_load(&mut self, budget: Duration) -> std::path::PathBuf {
        let cmd = self
            .expect_player_command(budget, |cmd| matches!(cmd, MockCommand::Load(..)))
            .await;
        let MockCommand::Load(path, _) = cmd else {
            unreachable!()
        };
        path
    }

    /// Wait (sim time) for a player command matching `pred`; commands
    /// before it are discarded.
    pub async fn expect_player_command<F: FnMut(&MockCommand) -> bool>(
        &mut self,
        budget: Duration,
        mut pred: F,
    ) -> MockCommand {
        let deadline = tokio::time::Instant::now() + budget;
        let mut seen = Vec::new();
        loop {
            while let Some(cmd) = self.control.try_command() {
                if pred(&cmd) {
                    return cmd;
                }
                seen.push(cmd);
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "{}: expected player command never arrived; saw {seen:#?}",
                    self.name
                );
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Wait for a local narrator line matching `pred`.
    pub async fn expect_system_line<F: FnMut(&str) -> bool>(
        &mut self,
        budget: Duration,
        mut pred: F,
    ) -> String {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            while let Ok(lines) = self.ui_lines.try_recv() {
                if let Some(notice) = lines.system.into_iter().find(|n| pred(&n.text)) {
                    return notice.text;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("{}: expected narrator line never arrived", self.name);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// A real (tiny) media file: contents on disk, true ed2k hash as the
/// playlist key — so the matcher and hash verification run for real.
pub struct MediaFile {
    pub hash: Ed2kHash,
    pub filename: String,
    pub contents: Vec<u8>,
}

/// A deterministic media file (distinct contents per index).
pub fn media_file(i: u8) -> MediaFile {
    let contents = format!("episode {i} contents").into_bytes();
    MediaFile {
        hash: dessplay_core::hash::ed2k_hash_bytes(&contents).root,
        filename: format!("ep{i}.mkv"),
        contents,
    }
}

/// A playlist entry for a [`MediaFile`].
pub fn file_entry(file: &MediaFile, added_by: &str) -> NewPlaylistEntry {
    NewPlaylistEntry {
        hash: file.hash,
        added_by: UserId::new(added_by),
        filename: file.filename.clone(),
        size_bytes: file.contents.len() as u64,
        duration_millis: Some(1_440_000),
    }
}

/// Anything snapshots can be read from ([`ClientHandle`] or
/// [`PlayerClient`]).
pub trait SnapshotSource {
    fn sync_tx(&self) -> &mpsc::Sender<SyncCommand>;
    fn network_tx(&self) -> &mpsc::Sender<NetworkCommand>;
    fn peers_rx(&self) -> &watch::Receiver<Vec<PeerInfo>>;
}

impl<S: SnapshotSource> SnapshotSource for &S {
    fn sync_tx(&self) -> &mpsc::Sender<SyncCommand> {
        (*self).sync_tx()
    }
    fn network_tx(&self) -> &mpsc::Sender<NetworkCommand> {
        (*self).network_tx()
    }
    fn peers_rx(&self) -> &watch::Receiver<Vec<PeerInfo>> {
        (*self).peers_rx()
    }
}

impl SnapshotSource for ClientHandle {
    fn sync_tx(&self) -> &mpsc::Sender<SyncCommand> {
        &self.sync
    }
    fn network_tx(&self) -> &mpsc::Sender<NetworkCommand> {
        &self.network
    }
    fn peers_rx(&self) -> &watch::Receiver<Vec<PeerInfo>> {
        &self.peers
    }
}

impl SnapshotSource for PlayerClient {
    fn sync_tx(&self) -> &mpsc::Sender<SyncCommand> {
        &self.sync
    }
    fn network_tx(&self) -> &mpsc::Sender<NetworkCommand> {
        &self.network
    }
    fn peers_rx(&self) -> &watch::Receiver<Vec<PeerInfo>> {
        &self.peers
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

pub async fn view_of<S: SnapshotSource>(handle: &S) -> StateView {
    let (tx, rx) = oneshot::channel();
    handle
        .sync_tx()
        .send(SyncCommand::GetView(tx))
        .await
        .unwrap();
    rx.await.unwrap()
}

pub async fn epoch_of<S: SnapshotSource>(handle: &S) -> Epoch {
    let (tx, rx) = oneshot::channel();
    handle
        .sync_tx()
        .send(SyncCommand::GetEpoch(tx))
        .await
        .unwrap();
    rx.await.unwrap()
}

pub async fn snapshot_of<S: SnapshotSource>(handle: &S) -> ClientSnapshot {
    ClientSnapshot {
        view: view_of(handle).await,
        peers: handle.peers_rx().borrow().clone(),
        epoch: epoch_of(handle).await,
    }
}

pub async fn mutate<S: SnapshotSource>(handle: &S, mutation: Mutation) {
    handle
        .sync_tx()
        .send(SyncCommand::Mutate(Box::new(mutation)))
        .await
        .unwrap();
}

/// Report end-of-file to the server, as the player layer will.
pub async fn report_eof<S: SnapshotSource>(handle: &S, file: Ed2kHash) {
    handle
        .network_tx()
        .send(NetworkCommand::SendReliable(Box::new(
            ServerControl::EofReached { file },
        )))
        .await
        .unwrap();
}

/// Manually mark a file's group watched flag, as the episode browser will.
pub async fn mark_watched<S: SnapshotSource>(handle: &S, file: Ed2kHash, watched: bool) {
    handle
        .network_tx()
        .send(NetworkCommand::SendReliable(Box::new(
            ServerControl::MarkWatched { file, watched },
        )))
        .await
        .unwrap();
}

/// Graceful quit: Goodbye + connection teardown.
pub async fn quit<S: SnapshotSource>(handle: &S) {
    handle
        .network_tx()
        .send(NetworkCommand::Shutdown)
        .await
        .unwrap();
}

/// Wait (in simulated time) until `pred` holds over all clients'
/// snapshots. The auto-waiting assertion from testing-strategy.md.
pub async fn eventually<S: SnapshotSource, F: FnMut(&[ClientSnapshot]) -> bool>(
    clients: &[&S],
    budget: Duration,
    mut pred: F,
) -> Vec<ClientSnapshot> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let mut snapshots = Vec::new();
        for client in clients {
            snapshots.push(snapshot_of(*client).await);
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
pub async fn eventually_views<S: SnapshotSource, F: FnMut(&[StateView]) -> bool>(
    clients: &[&S],
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

pub fn user_seek_authority(name: &str, event_at: u64) -> dessplay_core::types::SeekAuthority {
    dessplay_core::types::SeekAuthority::User(dessplay_core::types::UserSeek {
        user: UserId::new(name),
        file: hash(1),
        event_at: dessplay_core::types::SharedTimestamp(event_at),
        from_millis: 0,
        to_millis: 10_000,
    })
}

/// A full client + production `SessionLoop` against the sim server, no
/// terminal — the rig for flows that must run the real bridge loop
/// (UserAction handling, on-disk storage, transfer streams). Unlike
/// [`Harness::player_client`], its storage lives at a caller-owned
/// `db_dir`, so dropping the rig and calling [`loop_rig`] again with the
/// same directory models a client restart.
pub struct LoopRig {
    pub actions: mpsc::Sender<dessplay::ui::msg::UserAction>,
    /// UI inputs the loop pushed (kept alive — sends are lossy
    /// `try_send`s into this).
    pub ui_rx: std::sync::mpsc::Receiver<dessplay::ui::shell::UiInput>,
    pub sync: mpsc::Sender<SyncCommand>,
    pub task: tokio::task::JoinHandle<dessplay::run::SessionEnd>,
}

impl LoopRig {
    /// The rig's current synced view (its own replica).
    pub async fn view(&self) -> StateView {
        let (tx, rx) = oneshot::channel();
        self.sync
            .send(SyncCommand::GetView(tx))
            .await
            .expect("sync actor gone");
        rx.await.expect("sync actor gone")
    }

    /// Quit the session loop and wait for it to wind down.
    pub async fn quit(self) {
        self.actions
            .send(dessplay::ui::msg::UserAction::Quit)
            .await
            .expect("loop gone");
        tokio::time::timeout(Duration::from_secs(10), self.task)
            .await
            .expect("session loop did not quit")
            .expect("loop task panicked");
    }
}

/// Spawn a [`LoopRig`]. Real time only (`multi_thread` tests): hashing
/// and transfers live on the blocking pool, which paused time can't
/// drive.
pub fn loop_rig(harness: &Harness, name: &str, nonce: u128, db_dir: &std::path::Path) -> LoopRig {
    let handle = harness.client(name, nonce);
    let sync = handle.sync.clone();
    let cache_dir = db_dir.join(format!("{name}-cache"));
    std::fs::create_dir_all(&cache_dir).expect("cache dir");
    let shell = SessionShell::new(
        UserId::new(name),
        MockFactory::new([]),
        sim_clock(0),
        dessplay::actors::file::FileConfig {
            storage: dessplay::storage::Storage::open(&db_dir.join(format!("{name}-file.db")))
                .expect("opening file storage"),
            media_roots: vec![],
            retention: dessplay::config::CacheRetention::default(),
            cache_dir,
            clock: sim_clock(0),
            download: dessplay::download::DownloadConfig::default(),
            upload_limit: None,
            scan_interval: None,
            scan_transfer_quiet: dessplay::actors::file::SCAN_TRANSFER_QUIET_DEFAULT,
            torrent: None,
            nyaa: None,
        },
        true, // auto_download
        handle.sync.clone(),
        handle.network.clone(),
    );
    let storage = dessplay::storage::Storage::open(&db_dir.join(format!("{name}.db")))
        .expect("opening storage");
    let (action_tx, action_rx) = mpsc::channel(64);
    let (ui_tx, ui_rx) = std::sync::mpsc::sync_channel(64);
    // Inert IRC bridge: the opposite ends are dropped so it never connects.
    let (irc_tx, _irc_rx) = mpsc::channel(8);
    let (_irc_ev_tx, irc_events) = mpsc::channel(8);
    let mut session = dessplay::run::SessionLoop {
        handle,
        shell,
        actions: action_rx,
        ui: ui_tx,
        storage,
        db_path: db_dir.join(format!("{name}.db")),
        me: UserId::new(name),
        settings: dessplay::config::Settings::default(),
        media_roots: Vec::new(),
        observed_fingerprint: Box::new(|| None),
        pin_pending: false,
        server_addr: "sim".into(),
        start: std::time::Instant::now(),
        irc_tx,
        irc_events,
        irc_alive: true,
        link: Default::default(),
        transfer_link_down: false,
        torrent_engine: None,
        health: None,
        health_level: Default::default(),
        suggestion: None,
        advisor: Default::default(),
        commentary: dessplay::commentary::CommentaryEngine::disabled(),
        clipboard: None,
    };
    let task = tokio::spawn(async move { session.run().await });
    LoopRig {
        actions: action_tx,
        ui_rx,
        sync,
        task,
    }
}
