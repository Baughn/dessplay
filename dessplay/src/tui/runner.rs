use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::EventStream;
use tokio_stream::StreamExt;

use crate::app_state::{AppEffect, AppEvent, AppState};
use crate::media_scanner::MediaIndex;
use crate::series_browser;
use crate::peer_conn::PeerManager;
use crate::player::echo::EchoFilter;
use crate::player::mpv::MpvPlayer;
use crate::player::Player;
use crate::rendezvous_client::{RendezvousClient, RendezvousEvent};
use crate::storage::{ClientStorage, Config};
use dessplay_core::types::{FileId, SharedTimestamp};
use ratatui::layout::Rect;

use crate::tui::layout::compute_layout;
use crate::tui::terminal::{setup_file_logging, setup_terminal};
use crate::tui::ui_state::{
    FileBrowserOrigin, FileBrowserState, FocusedPane, HashingState, InputResult, InputState,
    MetadataAssignState, MetadataAssignStep, Screen, SeriesChoice, SettingsState, UiAction, UiState,
};
use crate::tls::TofuVerifier;
use crate::tui::ui_state::TofuWarningState;
use crate::tui::widgets::{
    chat, file_browser, hashing_progress, keybinding_bar, metadata_assign, player_status, playlist,
    recent_series, settings, tofu_warning, users,
};
use dessplay_core::framing::{
    read_framed, write_framed, TAG_GAP_FILL_REQUEST, TAG_GAP_FILL_RESPONSE,
};
use dessplay_core::network::NetworkEvent;
use dessplay_core::protocol::{
    CrdtOp, GapFillRequest, GapFillResponse, LwwValue, PeerControl, PeerDatagram, RvControl,
};
use dessplay_core::sync_engine::{SyncAction, SyncEngine};
use dessplay_core::types::{AniDbMetadata, FileState, MetadataSource, PeerId, UserId, UserState};

/// Tracks file mtimes and manual-map status for re-hash on unpause.
struct MtimeTracker {
    /// Loaded file mtimes: FileId → (path, last known mtime).
    mtimes: HashMap<FileId, (PathBuf, std::time::SystemTime)>,
    /// Files that were manually mapped (Ctrl-M) — skip mtime checks.
    manually_mapped: HashSet<FileId>,
}

impl MtimeTracker {
    fn new() -> Self {
        Self {
            mtimes: HashMap::new(),
            manually_mapped: HashSet::new(),
        }
    }

    /// Record the mtime of a file when it's loaded into the player.
    fn record_mtime(&mut self, file_id: FileId, path: &std::path::Path) {
        if let Ok(meta) = std::fs::metadata(path)
            && let Ok(mtime) = meta.modified()
        {
            self.mtimes.insert(file_id, (path.to_path_buf(), mtime));
        }
    }

    /// Check if a file's mtime has changed since it was recorded.
    /// Returns Some(path) if mtime changed and file is not manually mapped.
    fn check_mtime_changed(&self, file_id: &FileId) -> Option<PathBuf> {
        if self.manually_mapped.contains(file_id) {
            return None;
        }
        if let Some((path, recorded_mtime)) = self.mtimes.get(file_id)
            && let Ok(meta) = std::fs::metadata(path)
            && let Ok(current_mtime) = meta.modified()
            && current_mtime != *recorded_mtime
        {
            return Some(path.clone());
        }
        None
    }
}

/// Result of a background file hash operation.
struct BgHashResult {
    path: PathBuf,
    file_size: u64,
    result: std::io::Result<FileId>,
}

/// Shared progress counters for background media indexing.
struct BgHashProgress {
    total_files: usize,
    completed_files: Arc<AtomicU64>,
}

/// Background worker that hashes all media files not yet in file_mappings.
///
/// Yields to user-initiated hashes when `user_hash_active` is set.
async fn bg_hash_worker(
    paths: Vec<PathBuf>,
    user_hash_active: Arc<AtomicBool>,
    progress: Arc<BgHashProgress>,
    tx: tokio::sync::mpsc::UnboundedSender<BgHashResult>,
) {
    for path in paths {
        // Yield to user-initiated hashes
        while user_hash_active.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        let file_size = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(e) => {
                tracing::debug!(path = %path.display(), "Skipping bg hash, metadata error: {e}");
                progress.completed_files.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };

        let path_clone = path.clone();
        let result = tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(&path_clone)?;
            let reader = std::io::BufReader::new(file);
            dessplay_core::ed2k::compute_ed2k(reader)
        })
        .await;

        let hash_result = match result {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(path = %path.display(), "Bg hash task panicked: {e}");
                progress.completed_files.fetch_add(1, Ordering::Relaxed);
                continue;
            }
        };

        progress.completed_files.fetch_add(1, Ordering::Relaxed);

        if tx.send(BgHashResult { path, file_size, result: hash_result }).is_err() {
            break; // Receiver dropped, stop
        }

        // Small yield between files to avoid monopolizing the runtime
        tokio::task::yield_now().await;
    }
}

/// Main TUI entry point. Handles settings screen, connection, and event loop.
pub async fn run(storage: Arc<Mutex<ClientStorage>>, args: &[String]) -> Result<()> {
    // Set up file logging before entering TUI mode
    let _log_guard = setup_file_logging()?;

    let mut guard = setup_terminal()?;

    // Check if we have a config
    let config = {
        let s = storage.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        s.get_config()?
    };

    let mut ui = UiState::new();

    // If no config, show settings screen first
    if config.is_none() {
        ui = ui.with_settings();
        run_settings_screen(&mut guard.terminal, &mut ui, &storage).await?;
        ui.settings = None;
    }

    loop {
        // Reload config (may have been saved from settings screen)
        let config = {
            let s = storage.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            s.get_config()?
                .ok_or_else(|| anyhow::anyhow!("no config — settings not saved"))?
        };

        if ui.should_quit {
            return Ok(());
        }

        // Reset to main screen
        ui.screen = Screen::Main;
        ui.focus = FocusedPane::Chat;

        // Connect to server and run main loop
        let mut tofu_ref = None;
        match run_connected(
            &mut guard.terminal,
            &mut ui,
            &storage,
            &config,
            args,
            &mut tofu_ref,
        )
        .await
        {
            Ok(()) => {
                if ui.settings.is_some() {
                    // Settings saved from inline modal — reconnect with new config
                    ui.settings = None;
                    ui.screen = Screen::Main;
                    continue;
                }
                return Ok(());
            }
            Err(e) => {
                tracing::warn!("Connection error: {e:#}");

                if ui.should_quit {
                    return Ok(());
                }

                // Check for TOFU certificate mismatch
                if let Some(mismatch) = tofu_ref.and_then(|t| t.take_mismatch()) {
                    ui.tofu_warning = Some(TofuWarningState {
                        server: mismatch.server,
                        stored_fingerprint: mismatch.stored_fingerprint,
                        received_fingerprint: mismatch.received_fingerprint,
                    });
                    ui.screen = Screen::TofuWarning;

                    let accepted =
                        run_tofu_warning_screen(&mut guard.terminal, &mut ui, &storage).await?;

                    if ui.should_quit {
                        return Ok(());
                    }

                    if accepted {
                        // Delete stored cert and retry connection
                        let server = ui
                            .tofu_warning
                            .as_ref()
                            .map(|w| w.server.clone())
                            .unwrap_or_default();
                        if let Ok(s) = storage.lock() {
                            s.delete_cert(&server)?;
                        }
                        ui.tofu_warning = None;
                        continue;
                    }

                    ui.tofu_warning = None;
                    // Fall through to settings screen with error
                }

                // Re-open settings with the error as alert
                let media_roots = storage
                    .lock()
                    .ok()
                    .and_then(|s| s.get_media_roots().ok())
                    .unwrap_or_default();
                let mut settings = SettingsState::from_config(&config, media_roots);
                settings.alert = Some(format!("{e:#}"));

                ui.screen = Screen::Settings;
                ui.settings = Some(settings);
                run_settings_screen(&mut guard.terminal, &mut ui, &storage).await?;
            }
        }
    }
}

/// Run the settings screen loop until the user saves or quits.
async fn run_settings_screen(
    terminal: &mut crate::tui::terminal::Tui,
    ui: &mut UiState,
    storage: &Arc<Mutex<ClientStorage>>,
) -> Result<()> {
    let mut event_stream = EventStream::new();

    // Load existing media roots if any
    if let Some(ref mut settings) = ui.settings
        && let Ok(s) = storage.lock()
            && let Ok(roots) = s.get_media_roots() {
                settings.media_roots = roots;
            }

    loop {
        // Draw
        terminal.draw(|frame| {
            let wf = keybinding_bar::WindowFrame::new(frame.area());
            if let Some(ref settings_state) = ui.settings {
                settings::render_settings(wf.content, frame.buffer_mut(), settings_state);
            }
            wf.render_bar(frame.buffer_mut(), settings::keybindings());
        })?;

        // Handle file browser overlay if active
        if ui.file_browser.is_some() {
            run_file_browser_overlay(terminal, ui).await?;
            // Directory selection is handled inside the file browser overlay
            continue;
        }

        // Wait for input
        let Some(event) = event_stream.next().await else {
            break;
        };

        let event = event.context("failed to read terminal event")?;

        if let crossterm::event::Event::Key(key) = event {
            let result = crate::tui::input::handle_key_event(key, ui);
            match result {
                InputResult::UiAction(action) => {
                    apply_settings_action(ui, &action, storage)?;
                    if matches!(action, UiAction::SettingsSave)
                        && let Some(ref s) = ui.settings
                            && s.is_valid() {
                                return Ok(());
                            }
                    if matches!(action, UiAction::SettingsCancel) {
                        // In first-run/error context, cancel means quit
                        ui.should_quit = true;
                        return Ok(());
                    }
                }
                InputResult::None => {}
                _ => {}
            }
        }

        if ui.should_quit {
            return Ok(());
        }
    }

    Ok(())
}

/// Run the TOFU warning modal loop. Returns `true` if the user accepted the new certificate.
async fn run_tofu_warning_screen(
    terminal: &mut crate::tui::terminal::Tui,
    ui: &mut UiState,
    _storage: &Arc<Mutex<ClientStorage>>,
) -> Result<bool> {
    let mut event_stream = EventStream::new();

    loop {
        // Draw
        terminal.draw(|frame| {
            let wf = keybinding_bar::WindowFrame::new(frame.area());
            if let Some(ref warning_state) = ui.tofu_warning {
                tofu_warning::render_tofu_warning(wf.content, frame.buffer_mut(), warning_state);
            }
            wf.render_bar(frame.buffer_mut(), tofu_warning::keybindings());
        })?;

        // Wait for input
        let Some(event) = event_stream.next().await else {
            break;
        };

        let event = event.context("failed to read terminal event")?;

        if let crossterm::event::Event::Key(key) = event {
            let result = crate::tui::input::handle_key_event(key, ui);
            if let InputResult::UiAction(action) = result {
                match action {
                    UiAction::TofuAccept => return Ok(true),
                    UiAction::TofuReject => return Ok(false),
                    UiAction::Quit => {
                        ui.should_quit = true;
                        return Ok(false);
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(false)
}

fn apply_settings_action(
    ui: &mut UiState,
    action: &UiAction,
    storage: &Arc<Mutex<ClientStorage>>,
) -> Result<()> {
    match action {
        UiAction::Quit => {
            ui.should_quit = true;
        }
        UiAction::SettingsNextField => {
            if let Some(ref mut s) = ui.settings {
                s.next_field();
            }
        }
        UiAction::SettingsPrevField => {
            if let Some(ref mut s) = ui.settings {
                s.prev_field();
            }
        }
        UiAction::SettingsInsertChar(c) => {
            if let Some(ref mut s) = ui.settings {
                s.alert = None;
                match s.focused_field {
                    0 => s.username.push(*c),
                    1 => s.server.push(*c),
                    3 => s.password.push(*c),
                    _ => {}
                }
            }
        }
        UiAction::SettingsDeleteBack => {
            if let Some(ref mut s) = ui.settings {
                s.alert = None;
                match s.focused_field {
                    0 => { s.username.pop(); }
                    1 => { s.server.pop(); }
                    3 => { s.password.pop(); }
                    _ => {}
                }
            }
        }
        UiAction::SettingsTogglePlayer => {
            if let Some(ref mut s) = ui.settings {
                s.alert = None;
                s.player = if s.player == "mpv" {
                    "vlc".to_string()
                } else {
                    "mpv".to_string()
                };
            }
        }
        UiAction::SettingsAddMediaRoot => {
            // Open file browser for directory selection
            let start_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
            ui.file_browser = Some(FileBrowserState::open(
                start_dir,
                FileBrowserOrigin::SettingsMediaRoot,
            ));
            ui.screen = Screen::FileBrowser;
        }
        UiAction::SettingsRemoveMediaRoot => {
            if let Some(ref mut s) = ui.settings
                && !s.media_roots.is_empty() {
                    s.alert = None;
                    s.media_roots.pop();
                }
        }
        UiAction::SettingsMoveRootUp => {
            // For simplicity, swap last two roots (full reorder logic in future)
            if let Some(ref mut s) = ui.settings {
                let len = s.media_roots.len();
                if len >= 2 {
                    s.alert = None;
                    s.media_roots.swap(len - 2, len - 1);
                }
            }
        }
        UiAction::SettingsMoveRootDown => {
            if let Some(ref mut s) = ui.settings {
                let len = s.media_roots.len();
                if len >= 2 {
                    s.alert = None;
                    s.media_roots.swap(0, 1);
                }
            }
        }
        UiAction::SettingsSave => {
            if let Some(ref s) = ui.settings
                && s.is_valid() {
                    let config = Config {
                        username: s.username.trim().to_string(),
                        server: s.server.trim().to_string(),
                        player: s.player.clone(),
                        password: if s.password.is_empty() {
                            std::env::var("DESSPLAY_PASSWORD").ok()
                        } else {
                            Some(s.password.clone())
                        },
                    };
                    let roots: Vec<PathBuf> = s.media_roots.clone();
                    let st = storage.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
                    st.save_config(&config)?;
                    st.set_media_roots(&roots)?;
                }
        }
        UiAction::SettingsCancel => {
            ui.settings = None;
            ui.screen = Screen::Main;
        }
        _ => {}
    }
    Ok(())
}

/// Run the file browser as an overlay until selection or cancel.
async fn run_file_browser_overlay(
    terminal: &mut crate::tui::terminal::Tui,
    ui: &mut UiState,
) -> Result<()> {
    let mut event_stream = EventStream::new();

    loop {
        // Draw
        terminal.draw(|frame| {
            let wf = keybinding_bar::WindowFrame::new(frame.area());
            if let Some(ref fb) = ui.file_browser {
                file_browser::render_file_browser(wf.content, frame.buffer_mut(), fb);
                wf.render_bar(frame.buffer_mut(), &file_browser::keybindings(&fb.origin));
            }
        })?;

        let Some(event) = event_stream.next().await else {
            break;
        };
        let event = event.context("failed to read terminal event")?;

        if let crossterm::event::Event::Key(key) = event {
            let result = crate::tui::input::handle_key_event(key, ui);
            if let InputResult::UiAction(action) = result {
                match action {
                    UiAction::Quit => {
                        ui.should_quit = true;
                        return Ok(());
                    }
                    UiAction::FileBrowserUp => {
                        if let Some(ref mut fb) = ui.file_browser {
                            fb.select_up();
                        }
                    }
                    UiAction::FileBrowserDown => {
                        if let Some(ref mut fb) = ui.file_browser {
                            fb.select_down();
                        }
                    }
                    UiAction::FileBrowserSelect => {
                        let should_close = if let Some(ref mut fb) = ui.file_browser {
                            if let Some(entry) = fb.entries.get(fb.selected).cloned() {
                                if entry.is_dir {
                                    fb.current_dir = entry.path;
                                    fb.selected = 0;
                                    fb.scroll_offset = 0;
                                    fb.refresh_entries();
                                    false
                                } else {
                                    // File selected — will be handled by caller
                                    true
                                }
                            } else {
                                false
                            }
                        } else {
                            false
                        };

                        if should_close {
                            // For playlist file browser, the file path is captured
                            // For settings, this shouldn't happen (dirs only)
                            return Ok(());
                        }
                    }
                    UiAction::FileBrowserSelectDir => {
                        // Select the current directory (for media root selection)
                        if let Some(ref fb) = ui.file_browser
                            && fb.origin == FileBrowserOrigin::SettingsMediaRoot {
                                let dir = fb.current_dir.clone();
                                if let Some(ref mut settings) = ui.settings
                                    && !settings.media_roots.contains(&dir) {
                                        settings.media_roots.push(dir);
                                    }
                                ui.file_browser = None;
                                ui.screen = Screen::Settings;
                                return Ok(());
                            }
                    }
                    UiAction::FileBrowserBack => {
                        let should_close = if let Some(ref mut fb) = ui.file_browser {
                            if let Some(parent) = fb.current_dir.parent() {
                                // If already at root, close
                                if fb.current_dir == parent.to_path_buf() {
                                    true
                                } else {
                                    fb.current_dir = parent.to_path_buf();
                                    fb.selected = 0;
                                    fb.scroll_offset = 0;
                                    fb.refresh_entries();
                                    false
                                }
                            } else {
                                true
                            }
                        } else {
                            true
                        };

                        if should_close {
                            ui.file_browser = None;
                            // Return to previous screen
                            if ui.settings.is_some() {
                                ui.screen = Screen::Settings;
                            } else {
                                ui.screen = Screen::Main;
                            }
                            return Ok(());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

/// Run the connected main loop with TUI rendering.
async fn run_connected(
    terminal: &mut crate::tui::terminal::Tui,
    ui: &mut UiState,
    storage: &Arc<Mutex<ClientStorage>>,
    config: &Config,
    args: &[String],
    tofu_out: &mut Option<Arc<TofuVerifier>>,
) -> Result<()> {
    // Get password
    let password = config
        .password
        .clone()
        .or_else(|| std::env::var("DESSPLAY_PASSWORD").ok())
        .context("no password configured")?;

    let server_str = get_arg(args, "--server").unwrap_or_else(|| config.server.clone());

    let server_addr = tokio::net::lookup_host(&server_str)
        .await
        .context(format!("failed to resolve server address: {server_str}"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("server address resolved to no addresses: {server_str}"))?;

    let tofu = Arc::new(TofuVerifier::new(Arc::clone(storage), server_str.clone()));
    *tofu_out = Some(Arc::clone(&tofu));

    let bind_addr: std::net::SocketAddr = "[::]:0".parse().context("invalid bind address")?;
    let crate::quic::DualEndpoint {
        endpoint,
        peer_client_config,
    } = crate::quic::create_dual_endpoint(bind_addr, tofu)?;

    let rv_client = RendezvousClient::connect(
        &endpoint,
        server_addr,
        "dessplay-rendezvous",
        &password,
        &config.username,
    )
    .await?;

    tracing::info!(
        peer_id = %rv_client.peer_id,
        observed_addr = %rv_client.observed_addr,
        "Connected to rendezvous server"
    );

    let peer_mgr = Arc::new(PeerManager::new(
        endpoint,
        peer_client_config,
        rv_client.peer_id,
        config.username.clone(),
    ));
    peer_mgr.spawn_accept_loop();

    // Initialize AppState
    let app_state = {
        let s = storage.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
        let user_id = UserId(config.username.clone());
        match s.load_latest_snapshot()? {
            Some((epoch, snapshot)) => {
                let mut state = dessplay_core::crdt::CrdtState::new();
                state.load_snapshot(epoch, snapshot);
                for op in s.load_ops(epoch)? {
                    state.apply_op(&op);
                }
                tracing::info!(%epoch, "Loaded persisted CRDT state");
                let engine = SyncEngine::from_persisted(epoch, state, epoch);
                AppState::from_persisted(user_id, engine)
            }
            None => {
                tracing::info!("No persisted state, starting fresh");
                AppState::new(user_id)
            }
        }
    };
    let app_state = Arc::new(tokio::sync::Mutex::new(app_state));

    // Build media index from configured media roots
    let media_roots = storage
        .lock()
        .ok()
        .and_then(|s| s.get_media_roots().ok())
        .unwrap_or_default();
    let media_index = Arc::new(MediaIndex::scan(&media_roots));
    tracing::info!(files = media_index.file_count(), "Media index built");

    // Send initial state summary
    {
        let app = app_state.lock().await;
        rv_client.send(RvControl::StateSummary {
            versions: app.sync_engine.version_vectors(),
        });
    }

    let (gap_fill_tx, mut gap_fill_rx) =
        tokio::sync::mpsc::unbounded_channel::<(PeerId, GapFillResponse)>();

    // Channel for non-blocking file hash results (user-initiated)
    let (hash_tx, mut hash_rx) =
        tokio::sync::mpsc::unbounded_channel::<(PathBuf, std::io::Result<FileId>)>();

    // Background hash worker state
    let user_hash_active = Arc::new(AtomicBool::new(false));
    let (bg_hash_tx, mut bg_hash_rx) = tokio::sync::mpsc::unbounded_channel::<BgHashResult>();
    let bg_hash_progress = {
        // Collect paths not yet in file_mappings
        let known_paths: HashSet<PathBuf> = storage
            .lock()
            .ok()
            .and_then(|s| s.get_all_mapped_paths().ok())
            .unwrap_or_default();
        let all_paths = media_index.all_paths();
        let unknown_paths: Vec<PathBuf> = all_paths
            .into_iter()
            .filter(|p| !known_paths.contains(p.as_path()))
            .cloned()
            .collect();
        let total = unknown_paths.len();
        if total > 0 {
            let progress = Arc::new(BgHashProgress {
                total_files: total,
                completed_files: Arc::new(AtomicU64::new(0)),
            });
            let worker_progress = Arc::clone(&progress);
            let worker_active = Arc::clone(&user_hash_active);
            let worker_tx = bg_hash_tx.clone();
            tokio::spawn(bg_hash_worker(unknown_paths, worker_active, worker_progress, worker_tx));
            tracing::info!(total, "Spawned background hash worker");
            Some(progress)
        } else {
            None
        }
    };

    let mut summary_interval = tokio::time::interval(Duration::from_secs(1));
    let mut position_tick = tokio::time::interval(Duration::from_millis(100));
    let mut event_stream = EventStream::new();
    let mut needs_redraw = true;

    // Channel for mtime re-hash results: (file_id, new_hash_result)
    let (rehash_tx, mut rehash_rx) =
        tokio::sync::mpsc::unbounded_channel::<(FileId, std::io::Result<FileId>)>();

    // Player state
    let mut player: Option<MpvPlayer> = None;
    let mut echo_filter = EchoFilter::new();
    let mut mtime_tracker = MtimeTracker::new();

    // Try auto-matching all existing playlist items (after player is declared)
    {
        let now = rv_client.shared_now().await;
        let effects =
            try_auto_match_all(&app_state, &media_index, storage, now).await;
        dispatch_effects(
            effects, &peer_mgr, &rv_client, storage,
            &app_state, &gap_fill_tx,
            &mut player, &mut echo_filter,
            &mut mtime_tracker, &rehash_tx,
        ).await;
    }

    loop {
        // Draw if needed
        if needs_redraw {
            draw_main_screen(terminal, ui, &app_state, storage, &bg_hash_progress).await?;
            needs_redraw = false;
        }

        tokio::select! {
            // Terminal events
            event = event_stream.next() => {
                let Some(event) = event else { break; };
                let event = event.context("failed to read terminal event")?;

                match event {
                    crossterm::event::Event::Key(key) => {
                        if ui.screen == Screen::FileBrowser {
                            // File browser overlay handles its own events
                            run_file_browser_overlay(terminal, ui).await?;

                            if ui.should_quit {
                                break;
                            }

                            // Check if a file was selected (from playlist add)
                            let selected_file = ui.file_browser.as_ref().and_then(|fb| {
                                fb.entries.get(fb.selected).and_then(|entry| {
                                    if !entry.is_dir {
                                        Some(entry.path.clone())
                                    } else {
                                        None
                                    }
                                })
                            });

                            if let Some(path) = selected_file {
                                // Check if this is a manual map selection
                                let manual_map_info = ui.file_browser.as_ref().and_then(|fb| {
                                    if let FileBrowserOrigin::ManualMap {
                                        ref file_id,
                                        ..
                                    } = fb.origin
                                    {
                                        Some(*file_id)
                                    } else {
                                        None
                                    }
                                });

                                if let Some(file_id) = manual_map_info {
                                    // Manual map: store mapping, clear Missing, record dir
                                    ui.file_browser = None;
                                    ui.screen = Screen::Main;

                                    if let Ok(s) = storage.lock() {
                                        let _ = s.set_file_mapping(&file_id, &path);
                                    }
                                    // Mark as manually mapped — skip mtime checks
                                    mtime_tracker.manually_mapped.insert(file_id);
                                    // Record the directory for this series
                                    if let Some(dir) = path.parent() {
                                        let app = app_state.lock().await;
                                        let anime_id = app
                                            .sync_engine
                                            .state()
                                            .anidb
                                            .read(&file_id)
                                            .and_then(|opt| opt.as_ref())
                                            .map(|meta| meta.anime_id);
                                        drop(app);
                                        if let Some(aid) = anime_id
                                            && let Ok(s) = storage.lock()
                                        {
                                            let _ = s.set_series_mapping_dir(aid, dir);
                                        }
                                    }
                                    // Set file state to Ready
                                    let now = rv_client.shared_now().await;
                                    let effects = app_state.lock().await.process_event(
                                        AppEvent::SetFileState {
                                            file_id,
                                            state: dessplay_core::types::FileState::Ready,
                                        },
                                        now,
                                    );
                                    dispatch_effects(
                                        effects, &peer_mgr, &rv_client, storage,
                                        &app_state, &gap_fill_tx,
                                        &mut player, &mut echo_filter,
                                        &mut mtime_tracker, &rehash_tx,
                                    ).await;
                                    tracing::info!(?file_id, path = %path.display(), "Manual map stored");
                                } else {
                                    ui.file_browser = None;

                                    // Normal playlist add: hash and add
                                    let total_bytes = std::fs::metadata(&path)
                                        .map(|m| m.len())
                                        .unwrap_or(0);
                                    let progress = Arc::new(AtomicU64::new(0));
                                    let filename = path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| path.display().to_string());

                                    ui.hashing = Some(HashingState {
                                        filename,
                                        total_bytes,
                                        bytes_hashed: Arc::clone(&progress),
                                    });
                                    ui.screen = Screen::Hashing;

                                    // Pause background hashing during user hash
                                    user_hash_active.store(true, Ordering::Relaxed);

                                    // Spawn non-blocking hash
                                    let tx = hash_tx.clone();
                                    let path_for_hash = path.clone();
                                    tokio::task::spawn_blocking(move || {
                                        let result = (|| {
                                            let file = std::fs::File::open(&path_for_hash)?;
                                            let reader = std::io::BufReader::new(file);
                                            dessplay_core::ed2k::compute_ed2k_with_progress(
                                                reader,
                                                |bytes| progress.store(bytes, Ordering::Relaxed),
                                            )
                                        })();
                                        let _ = tx.send((path_for_hash, result));
                                    });
                                }
                            } else if ui.file_browser.is_none() {
                                // File browser was closed without selection
                                // Return to settings if open, otherwise main
                                if ui.settings.is_some() {
                                    ui.screen = Screen::Settings;
                                } else {
                                    ui.screen = Screen::Main;
                                }
                            }

                            needs_redraw = true;
                            continue;
                        }

                        let result = crate::tui::input::handle_key_event(key, ui);
                        match result {
                            InputResult::AppEvent(event) => {
                                if ui.screen != Screen::Settings {
                                    let now = rv_client.shared_now().await;
                                    let effects = app_state.lock().await.process_event(event, now);
                                    dispatch_effects(
                                        effects, &peer_mgr, &rv_client, storage,
                                        &app_state, &gap_fill_tx,
                                        &mut player, &mut echo_filter,
                                        &mut mtime_tracker, &rehash_tx,
                                    ).await;
                                }
                                needs_redraw = true;
                            }
                            InputResult::UiAction(action) => {
                                if ui.screen == Screen::Settings {
                                    apply_settings_action(ui, &action, storage)?;
                                    if matches!(action, UiAction::SettingsSave)
                                        && ui.settings.as_ref().is_some_and(|s| s.is_valid())
                                    {
                                        // Settings saved — break to reconnect with new config
                                        break;
                                    }
                                    if matches!(action, UiAction::SettingsCancel) {
                                        ui.settings = None;
                                        ui.screen = Screen::Main;
                                    }
                                } else {
                                    apply_main_ui_action(ui, &action, storage, &app_state, &rv_client).await?;
                                }
                                needs_redraw = true;
                            }
                            InputResult::Both(event, action) => {
                                if ui.screen == Screen::Settings {
                                    apply_settings_action(ui, &action, storage)?;
                                    if matches!(action, UiAction::SettingsSave)
                                        && ui.settings.as_ref().is_some_and(|s| s.is_valid())
                                    {
                                        break;
                                    }
                                    if matches!(action, UiAction::SettingsCancel) {
                                        ui.settings = None;
                                        ui.screen = Screen::Main;
                                    }
                                } else {
                                    let now = rv_client.shared_now().await;
                                    let effects = app_state.lock().await.process_event(event, now);
                                    dispatch_effects(
                                        effects, &peer_mgr, &rv_client, storage,
                                        &app_state, &gap_fill_tx,
                                        &mut player, &mut echo_filter,
                                        &mut mtime_tracker, &rehash_tx,
                                    ).await;
                                    apply_main_ui_action(ui, &action, storage, &app_state, &rv_client).await?;
                                }
                                needs_redraw = true;
                            }
                            InputResult::None => {}
                        }
                    }
                    crossterm::event::Event::Resize(_, _) => {
                        needs_redraw = true;
                    }
                    _ => {}
                }

                if ui.should_quit {
                    break;
                }
            }

            // Rendezvous server events
            event = rv_client.recv() => {
                let now = rv_client.shared_now().await;
                match event {
                    Some(RendezvousEvent::PeerList { peers }) => {
                        tracing::info!(count = peers.len(), "Got peer list update");
                        peer_mgr.update_peer_list(peers).await;
                    }
                    Some(RendezvousEvent::StateOp { op }) => {
                        let triggers_auto_match = matches!(
                            &op,
                            dessplay_core::protocol::CrdtOp::LwwWrite {
                                value: LwwValue::FileName(..)
                                    | LwwValue::AniDb(..),
                                ..
                            } | dessplay_core::protocol::CrdtOp::PlaylistOp { .. }
                        );
                        let effects = app_state.lock().await.process_event(
                            AppEvent::RemoteOp { from: PeerId(0), op },
                            now,
                        );
                        dispatch_effects(
                            effects, &peer_mgr, &rv_client, storage,
                            &app_state, &gap_fill_tx,
                            &mut player, &mut echo_filter,
                            &mut mtime_tracker, &rehash_tx,
                        ).await;
                        if triggers_auto_match {
                            let match_effects =
                                try_auto_match_all(&app_state, &media_index, storage, now).await;
                            dispatch_effects(
                                match_effects, &peer_mgr, &rv_client, storage,
                                &app_state, &gap_fill_tx,
                                &mut player, &mut echo_filter,
                                &mut mtime_tracker, &rehash_tx,
                            ).await;
                        }
                    }
                    Some(RendezvousEvent::StateSummary { versions }) => {
                        let effects = app_state.lock().await.process_event(
                            AppEvent::StateSummary {
                                from: PeerId(0),
                                epoch: versions.epoch,
                                versions,
                            },
                            now,
                        );
                        dispatch_effects(
                            effects, &peer_mgr, &rv_client, storage,
                            &app_state, &gap_fill_tx,
                            &mut player, &mut echo_filter,
                            &mut mtime_tracker, &rehash_tx,
                        ).await;
                    }
                    Some(RendezvousEvent::StateSnapshot { epoch, crdts }) => {
                        let effects = app_state.lock().await.process_event(
                            AppEvent::StateSnapshot { epoch, snapshot: crdts },
                            now,
                        );
                        dispatch_effects(
                            effects, &peer_mgr, &rv_client, storage,
                            &app_state, &gap_fill_tx,
                            &mut player, &mut echo_filter,
                            &mut mtime_tracker, &rehash_tx,
                        ).await;
                        let match_effects =
                            try_auto_match_all(&app_state, &media_index, storage, now).await;
                        dispatch_effects(
                            match_effects, &peer_mgr, &rv_client, storage,
                            &app_state, &gap_fill_tx,
                            &mut player, &mut echo_filter,
                            &mut mtime_tracker, &rehash_tx,
                        ).await;
                    }
                    None => {
                        tracing::info!("Rendezvous server disconnected");
                        break;
                    }
                }
                needs_redraw = true;
            }

            // Peer events
            event = peer_mgr.recv() => {
                let now = rv_client.shared_now().await;
                match event {
                    Ok(NetworkEvent::PeerConnected { peer_id, username }) => {
                        tracing::info!(%peer_id, %username, "Peer connected");
                        let effects = app_state.lock().await.process_event(
                            AppEvent::PeerConnected { peer_id, username },
                            now,
                        );
                        dispatch_effects(
                            effects, &peer_mgr, &rv_client, storage,
                            &app_state, &gap_fill_tx,
                            &mut player, &mut echo_filter,
                            &mut mtime_tracker, &rehash_tx,
                        ).await;
                    }
                    Ok(NetworkEvent::PeerDisconnected { peer_id }) => {
                        tracing::info!(%peer_id, "Peer disconnected");
                        let effects = app_state.lock().await.process_event(
                            AppEvent::PeerDisconnected { peer_id },
                            now,
                        );
                        dispatch_effects(
                            effects, &peer_mgr, &rv_client, storage,
                            &app_state, &gap_fill_tx,
                            &mut player, &mut echo_filter,
                            &mut mtime_tracker, &rehash_tx,
                        ).await;
                    }
                    Ok(NetworkEvent::PeerControl { from, message }) => {
                        let triggers_auto_match = matches!(
                            &message,
                            PeerControl::StateOp {
                                op: dessplay_core::protocol::CrdtOp::LwwWrite {
                                    value: LwwValue::FileName(..)
                                        | LwwValue::AniDb(..),
                                    ..
                                }
                            } | PeerControl::StateOp {
                                op: dessplay_core::protocol::CrdtOp::PlaylistOp { .. }
                            } | PeerControl::StateSnapshot { .. }
                        );
                        let app_event = match message {
                            PeerControl::StateOp { op } => {
                                Some(AppEvent::RemoteOp { from, op })
                            }
                            PeerControl::StateSummary { epoch, versions } => {
                                Some(AppEvent::StateSummary { from, epoch, versions })
                            }
                            PeerControl::StateSnapshot { epoch, crdts } => {
                                Some(AppEvent::StateSnapshot { epoch, snapshot: crdts })
                            }
                            other => {
                                tracing::debug!(%from, ?other, "Unhandled peer control");
                                None
                            }
                        };
                        if let Some(event) = app_event {
                            let effects = app_state.lock().await.process_event(event, now);
                            dispatch_effects(
                                effects, &peer_mgr, &rv_client, storage,
                                &app_state, &gap_fill_tx,
                                &mut player, &mut echo_filter,
                                &mut mtime_tracker, &rehash_tx,
                            ).await;
                        }
                        if triggers_auto_match {
                            let match_effects =
                                try_auto_match_all(&app_state, &media_index, storage, now).await;
                            dispatch_effects(
                                match_effects, &peer_mgr, &rv_client, storage,
                                &app_state, &gap_fill_tx,
                                &mut player, &mut echo_filter,
                                &mut mtime_tracker, &rehash_tx,
                            ).await;
                        }
                    }
                    Ok(NetworkEvent::PeerDatagram { from, message }) => {
                        let triggers_auto_match = matches!(
                            &message,
                            PeerDatagram::StateOp {
                                op: dessplay_core::protocol::CrdtOp::LwwWrite {
                                    value: LwwValue::FileName(..)
                                        | LwwValue::AniDb(..),
                                    ..
                                }
                            } | PeerDatagram::StateOp {
                                op: dessplay_core::protocol::CrdtOp::PlaylistOp { .. }
                            }
                        );
                        let app_event = match message {
                            PeerDatagram::StateOp { op } => {
                                Some(AppEvent::RemoteOp { from, op })
                            }
                            PeerDatagram::Position { position_secs, .. } => {
                                Some(AppEvent::RemotePosition { from, position_secs })
                            }
                            PeerDatagram::Seek { target_secs, .. } => {
                                Some(AppEvent::RemoteSeek { from, target_secs })
                            }
                        };
                        if let Some(event) = app_event {
                            let effects = app_state.lock().await.process_event(event, now);
                            dispatch_effects(
                                effects, &peer_mgr, &rv_client, storage,
                                &app_state, &gap_fill_tx,
                                &mut player, &mut echo_filter,
                                &mut mtime_tracker, &rehash_tx,
                            ).await;
                        }
                        if triggers_auto_match {
                            let match_effects =
                                try_auto_match_all(&app_state, &media_index, storage, now).await;
                            dispatch_effects(
                                match_effects, &peer_mgr, &rv_client, storage,
                                &app_state, &gap_fill_tx,
                                &mut player, &mut echo_filter,
                                &mut mtime_tracker, &rehash_tx,
                            ).await;
                        }
                    }
                    Ok(NetworkEvent::IncomingStream { from, stream }) => {
                        tracing::debug!(%from, "Incoming stream");
                        let app = Arc::clone(&app_state);
                        tokio::spawn(async move {
                            if let Err(e) = handle_incoming_stream(stream, app).await {
                                tracing::debug!(%from, "Gap fill stream error: {e}");
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Peer manager error: {e}");
                        break;
                    }
                }
                needs_redraw = true;
            }

            // Gap fill responses
            result = gap_fill_rx.recv() => {
                if let Some((from, response)) = result {
                    let now = rv_client.shared_now().await;
                    let effects = app_state.lock().await.process_event(
                        AppEvent::GapFillResponse { from, response },
                        now,
                    );
                    dispatch_effects(
                        effects, &peer_mgr, &rv_client, storage,
                        &app_state, &gap_fill_tx,
                        &mut player, &mut echo_filter,
                        &mut mtime_tracker, &rehash_tx,
                    ).await;
                    needs_redraw = true;
                }
            }

            // File hash completed (user-initiated)
            result = hash_rx.recv() => {
                if let Some((path, hash_result)) = result {
                    // Resume background hashing
                    user_hash_active.store(false, Ordering::Relaxed);

                    let file_size = ui.hashing.as_ref().map(|h| h.total_bytes).unwrap_or(0);
                    ui.hashing = None;
                    ui.screen = Screen::Main;

                    match hash_result {
                        Ok(file_id) => {
                            if let Ok(s) = storage.lock() {
                                let _ = s.set_file_mapping(&file_id, &path);
                            }
                            // Request AniDB metadata lookup from server
                            rv_client.send(dessplay_core::protocol::RvControl::AniDbLookup {
                                file_id,
                                file_size,
                            });
                            let now = rv_client.shared_now().await;
                            // Share the filename via CRDT
                            let filename = path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            if !filename.is_empty() {
                                let filename_op = CrdtOp::LwwWrite {
                                    timestamp: now,
                                    value: LwwValue::FileName(file_id, filename),
                                };
                                let fn_effects = app_state.lock().await
                                    .sync_engine.apply_local_op(filename_op);
                                dispatch_effects(
                                    vec![AppEffect::Sync(fn_effects)],
                                    &peer_mgr, &rv_client, storage,
                                    &app_state, &gap_fill_tx,
                                    &mut player, &mut echo_filter,
                                    &mut mtime_tracker, &rehash_tx,
                                ).await;
                            }
                            let effects = app_state.lock().await.process_event(
                                AppEvent::AddToPlaylist { file_id, after: None },
                                now,
                            );
                            dispatch_effects(
                                effects, &peer_mgr, &rv_client, storage,
                                &app_state, &gap_fill_tx,
                                &mut player, &mut echo_filter,
                                &mut mtime_tracker, &rehash_tx,
                            ).await;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to hash file: {e}");
                            ui.status_message = Some(format!("Hash error: {e}"));
                        }
                    }
                    needs_redraw = true;
                }
            }

            // Mtime re-hash completed
            result = rehash_rx.recv() => {
                if let Some((expected_file_id, hash_result)) = result {
                    match hash_result {
                        Ok(actual_file_id) => {
                            if actual_file_id == expected_file_id {
                                // Hash matches — file content unchanged, update stored mtime
                                tracing::info!(?expected_file_id, "Re-hash matches, mtime updated");
                                if let Some((path, _)) = mtime_tracker.mtimes.get(&expected_file_id) {
                                    let path = path.clone();
                                    mtime_tracker.record_mtime(expected_file_id, &path);
                                }
                                // Proceed with the deferred unpause
                                if let Some(p) = player.as_ref() {
                                    echo_filter.register_unpause();
                                    if let Err(e) = p.unpause().await {
                                        tracing::debug!("Failed to unpause player after re-hash: {e}");
                                    }
                                }
                            } else {
                                // Hash mismatch — file was replaced, set FileState::Missing
                                tracing::warn!(
                                    ?expected_file_id, ?actual_file_id,
                                    "Re-hash mismatch: file content changed"
                                );
                                let now = rv_client.shared_now().await;
                                let effects = app_state.lock().await.process_event(
                                    AppEvent::SetFileState {
                                        file_id: expected_file_id,
                                        state: FileState::Missing,
                                    },
                                    now,
                                );
                                dispatch_effects(
                                    effects, &peer_mgr, &rv_client, storage,
                                    &app_state, &gap_fill_tx,
                                    &mut player, &mut echo_filter,
                                    &mut mtime_tracker, &rehash_tx,
                                ).await;
                                // Remove the stale mtime entry
                                mtime_tracker.mtimes.remove(&expected_file_id);
                            }
                        }
                        Err(e) => {
                            tracing::warn!(?expected_file_id, "Re-hash failed: {e}");
                            // Can't verify — set Missing to be safe
                            let now = rv_client.shared_now().await;
                            let effects = app_state.lock().await.process_event(
                                AppEvent::SetFileState {
                                    file_id: expected_file_id,
                                    state: FileState::Missing,
                                },
                                now,
                            );
                            dispatch_effects(
                                effects, &peer_mgr, &rv_client, storage,
                                &app_state, &gap_fill_tx,
                                &mut player, &mut echo_filter,
                                &mut mtime_tracker, &rehash_tx,
                            ).await;
                            mtime_tracker.mtimes.remove(&expected_file_id);
                        }
                    }
                    needs_redraw = true;
                }
            }

            // Background hash completed
            result = bg_hash_rx.recv() => {
                if let Some(BgHashResult { path, file_size, result: hash_result }) = result {
                    match hash_result {
                        Ok(file_id) => {
                            if let Ok(s) = storage.lock() {
                                let _ = s.set_file_mapping(&file_id, &path);
                            }
                            // Request AniDB metadata lookup from server
                            rv_client.send(RvControl::AniDbLookup { file_id, file_size });
                            let now = rv_client.shared_now().await;
                            // Share filename via CRDT
                            let filename = path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            if !filename.is_empty() {
                                let filename_op = CrdtOp::LwwWrite {
                                    timestamp: now,
                                    value: LwwValue::FileName(file_id, filename),
                                };
                                let fn_effects = app_state.lock().await
                                    .sync_engine.apply_local_op(filename_op);
                                dispatch_effects(
                                    vec![AppEffect::Sync(fn_effects)],
                                    &peer_mgr, &rv_client, storage,
                                    &app_state, &gap_fill_tx,
                                    &mut player, &mut echo_filter,
                                    &mut mtime_tracker, &rehash_tx,
                                ).await;
                            }
                            // Try auto-matching any pending playlist items
                            let now = rv_client.shared_now().await;
                            let match_effects =
                                try_auto_match_all(&app_state, &media_index, storage, now).await;
                            dispatch_effects(
                                match_effects, &peer_mgr, &rv_client, storage,
                                &app_state, &gap_fill_tx,
                                &mut player, &mut echo_filter,
                                &mut mtime_tracker, &rehash_tx,
                            ).await;
                        }
                        Err(e) => {
                            tracing::debug!(path = %path.display(), "Background hash failed: {e}");
                        }
                    }
                    needs_redraw = true;
                }
            }

            // Player events
            event = async {
                if let Some(p) = player.as_ref() {
                    p.recv_event().await
                } else {
                    // No player — pend forever
                    std::future::pending::<anyhow::Result<crate::player::PlayerEvent>>().await
                }
            } => {
                if let Ok(raw_event) = event {
                    // Run through echo filter
                    if let Some(filtered) = echo_filter.filter(raw_event) {
                        let app_event = match filtered {
                            crate::player::PlayerEvent::Paused => AppEvent::PlayerPaused,
                            crate::player::PlayerEvent::Unpaused => AppEvent::PlayerUnpaused,
                            crate::player::PlayerEvent::Seeked { position_secs } => {
                                AppEvent::PlayerSeeked { position_secs }
                            }
                            crate::player::PlayerEvent::Position { position_secs } => {
                                AppEvent::PlayerPosition { position_secs }
                            }
                            crate::player::PlayerEvent::Duration { duration_secs } => {
                                AppEvent::PlayerDuration { duration_secs }
                            }
                            crate::player::PlayerEvent::Eof => AppEvent::PlayerEof,
                            crate::player::PlayerEvent::Crashed => AppEvent::PlayerCrashed,
                        };
                        let now = rv_client.shared_now().await;
                        let effects = app_state.lock().await.process_event(app_event, now);
                        dispatch_effects(
                            effects, &peer_mgr, &rv_client, storage,
                            &app_state, &gap_fill_tx,
                            &mut player, &mut echo_filter,
                            &mut mtime_tracker, &rehash_tx,
                        ).await;
                        needs_redraw = true;
                    }
                } else {
                    // Player channel closed — player crashed
                    let now = rv_client.shared_now().await;
                    let effects = app_state.lock().await.process_event(AppEvent::PlayerCrashed, now);
                    dispatch_effects(
                        effects, &peer_mgr, &rv_client, storage,
                        &app_state, &gap_fill_tx,
                        &mut player, &mut echo_filter,
                        &mut mtime_tracker, &rehash_tx,
                    ).await;
                    needs_redraw = true;
                }
            }

            // Position poll tick (100ms)
            _ = position_tick.tick() => {
                // Redraw during hashing to update the progress bar
                if ui.hashing.is_some() {
                    needs_redraw = true;
                }
                // Redraw during background indexing to update the counter
                if bg_hash_progress.as_ref().is_some_and(|p| {
                    p.completed_files.load(Ordering::Relaxed) < p.total_files as u64
                }) {
                    needs_redraw = true;
                }
            }

            // Periodic tick
            _ = summary_interval.tick() => {
                let now = rv_client.shared_now().await;
                let effects = app_state.lock().await.process_event(AppEvent::Tick, now);
                dispatch_effects(
                    effects, &peer_mgr, &rv_client, storage,
                    &app_state, &gap_fill_tx,
                    &mut player, &mut echo_filter,
                    &mut mtime_tracker, &rehash_tx,
                ).await;
                // Don't redraw on every tick unless effects requested it
            }
        }
    }

    // Clean up player on exit
    if let Some(p) = player {
        let _ = p.quit().await;
    }

    Ok(())
}

/// Draw the main screen.
async fn draw_main_screen(
    terminal: &mut crate::tui::terminal::Tui,
    ui: &UiState,
    app_state: &Arc<tokio::sync::Mutex<AppState>>,
    storage: &Arc<Mutex<ClientStorage>>,
    bg_hash_progress: &Option<Arc<BgHashProgress>>,
) -> Result<()> {
    // Collect all data from app state into owned values, then drop the lock
    let (
        chat_msgs,
        user_entries,
        playlist_entries,
        series_entries,
        current_file_name,
        position_secs,
        duration_secs,
        is_playing,
        blocking_users,
    ) = {
        let app = app_state.lock().await;
        let crdt = app.sync_engine.state();

        // Chat messages → owned
        let chat_view = crdt.chat.merged_view();
        let chat_msgs: Vec<(UserId, String)> = chat_view
            .iter()
            .map(|(uid, entry)| ((*uid).clone(), entry.text.clone()))
            .collect();

        // User entries
        let mut user_entries = Vec::new();

        let our_user_state = crdt
            .user_states
            .read(&app.our_user_id)
            .copied()
            .unwrap_or(UserState::Ready);
        let our_file_state = app
            .playback
            .current_file
            .and_then(|fid| crdt.file_states.read(&(app.our_user_id.clone(), fid)).cloned())
            .unwrap_or(FileState::Ready);
        user_entries.push(users::UserEntry {
            name: app.our_user_id.0.clone(),
            user_state: our_user_state,
            file_state: our_file_state,
            is_self: true,
        });

        for user_id in app.connected_peers.values() {
            let user_state = crdt
                .user_states
                .read(user_id)
                .copied()
                .unwrap_or(UserState::Ready);
            let file_state = app
                .playback
                .current_file
                .and_then(|fid| crdt.file_states.read(&(user_id.clone(), fid)).cloned())
                .unwrap_or(FileState::Ready);
            user_entries.push(users::UserEntry {
                name: user_id.0.clone(),
                user_state,
                file_state,
                is_self: false,
            });
        }

        // Playlist entries
        let playlist_snapshot = crdt.playlist.snapshot();
        let playlist_entries: Vec<playlist::PlaylistEntry> = playlist_snapshot
            .iter()
            .enumerate()
            .map(|(i, file_id)| {
                let local_path = storage
                    .lock()
                    .ok()
                    .and_then(|s| s.get_file_mapping(file_id).ok().flatten());
                let crdt_filename = crdt.filenames.read(file_id).cloned();
                let display_name =
                    playlist::file_display_name(file_id, local_path.as_deref(), crdt_filename.as_deref());
                let is_missing = local_path.is_none();
                playlist::PlaylistEntry {
                    file_id: *file_id,
                    display_name,
                    is_missing,
                    is_current: i == 0,
                }
            })
            .collect();

        // Series list for Recent Series pane
        let series_entries = {
            let st = storage.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            series_browser::build_series_list(crdt, &st)
        };

        let current_file_name: Option<String> =
            playlist_entries.first().map(|e| e.display_name.clone());

        let position_secs = app.our_position_secs;
        let duration_secs = app.file_duration_secs;
        let is_playing = app.playback.should_play;
        let blocking: Vec<String> = app
            .playback
            .blocking_users
            .iter()
            .map(|u| u.0.clone())
            .collect();

        (
            chat_msgs,
            user_entries,
            playlist_entries,
            series_entries,
            current_file_name,
            position_secs,
            duration_secs,
            is_playing,
            blocking,
        )
    };

    terminal.draw(|frame| {
        let area = frame.area();
        let layout = compute_layout(area);

        // Chat
        let chat_focused = ui.focus == crate::tui::ui_state::FocusedPane::Chat;
        let chat_refs: Vec<(&UserId, &str)> = chat_msgs
            .iter()
            .map(|(uid, text)| (uid, text.as_str()))
            .collect();
        chat::render_chat_messages(
            layout.chat_messages,
            frame.buffer_mut(),
            &chat_refs,
            ui.chat_scroll,
            chat_focused,
        );
        chat::render_chat_input(
            layout.chat_input,
            frame.buffer_mut(),
            &ui.input,
            chat_focused,
        );

        // Recent series
        recent_series::render_recent_series(
            layout.recent_series,
            frame.buffer_mut(),
            &series_entries,
            ui.recent_selected,
            ui.focus == crate::tui::ui_state::FocusedPane::RecentSeries,
        );

        // Users
        users::render_users(
            layout.users,
            frame.buffer_mut(),
            &user_entries,
            false, // users pane is never focused
        );

        // Playlist
        playlist::render_playlist(
            layout.playlist,
            frame.buffer_mut(),
            &playlist_entries,
            ui.playlist_selected,
            ui.focus == crate::tui::ui_state::FocusedPane::Playlist,
        );

        // Player status
        let bg_progress = bg_hash_progress.as_ref().and_then(|p| {
            let completed = p.completed_files.load(Ordering::Relaxed);
            let total = p.total_files;
            if completed < total as u64 {
                Some((completed, total))
            } else {
                None
            }
        });
        player_status::render_player_status(
            layout.player_status,
            frame.buffer_mut(),
            current_file_name.as_deref(),
            position_secs,
            duration_secs,
            is_playing,
            &blocking_users,
            bg_progress,
        );

        // Hashing progress modal overlay
        if let Some(ref hashing_state) = ui.hashing {
            hashing_progress::render_hashing_progress(area, frame.buffer_mut(), hashing_state);
        }

        // Settings modal overlay (if open)
        if let Some(ref settings_state) = ui.settings {
            // Modal: full width minus 4 padding, leaves 3 chat lines visible at bottom
            let modal_height = layout.chat_messages.height.saturating_sub(3);
            if modal_height > 0 {
                let modal_area = Rect {
                    x: area.x + 2,
                    y: area.y,
                    width: area.width.saturating_sub(4),
                    height: modal_height,
                };
                settings::render_settings(modal_area, frame.buffer_mut(), settings_state);
            }
            // Override keybinding bar with settings keybindings
            keybinding_bar::render_bar(
                layout.keybinding_bar,
                frame.buffer_mut(),
                settings::keybindings(),
            );
        } else if let Some(ref ma_state) = ui.metadata_assign {
            // Metadata assignment modal overlay (centered)
            let modal_height = area.height.min(20);
            let modal_width = area.width.saturating_sub(4).min(60);
            let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
            let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
            let modal_area = Rect {
                x,
                y,
                width: modal_width,
                height: modal_height,
            };
            metadata_assign::render_metadata_assign(modal_area, frame.buffer_mut(), ma_state);
            keybinding_bar::render_bar(
                layout.keybinding_bar,
                frame.buffer_mut(),
                &metadata_assign::keybindings(&ma_state.step),
            );
        } else {
            // Keybinding bar
            keybinding_bar::render_keybinding_bar(
                layout.keybinding_bar,
                frame.buffer_mut(),
                &ui.focus,
            );
        }
    })?;

    Ok(())
}

/// Apply a UI action in the main screen context.
async fn apply_main_ui_action(
    ui: &mut UiState,
    action: &UiAction,
    storage: &Arc<Mutex<ClientStorage>>,
    app_state: &Arc<tokio::sync::Mutex<AppState>>,
    rv_client: &RendezvousClient,
) -> Result<()> {
    match action {
        UiAction::Quit => {
            ui.should_quit = true;
        }
        UiAction::CycleFocus => {
            ui.focus = ui.focus.next();
        }
        // Chat input
        UiAction::InsertChar(c) => ui.input.insert_char(*c),
        UiAction::DeleteBack => ui.input.delete_back(),
        UiAction::DeleteForward => ui.input.delete_forward(),
        UiAction::CursorLeft => ui.input.move_left(),
        UiAction::CursorRight => ui.input.move_right(),
        UiAction::CursorWordLeft => ui.input.move_word_left(),
        UiAction::CursorWordRight => ui.input.move_word_right(),
        UiAction::CursorHome => ui.input.move_home(),
        UiAction::CursorEnd => ui.input.move_end(),
        UiAction::ClearInput => ui.input.clear(),
        UiAction::ScrollChatUp => {
            ui.chat_scroll = ui.chat_scroll.saturating_add(3);
        }
        UiAction::ScrollChatDown => {
            ui.chat_scroll = ui.chat_scroll.saturating_sub(3);
        }
        // Playlist
        UiAction::PlaylistSelectUp => {
            ui.playlist_selected = ui.playlist_selected.saturating_sub(1);
        }
        UiAction::PlaylistSelectDown => {
            let app = app_state.lock().await;
            let playlist_len = app.sync_engine.state().playlist.snapshot().len();
            if playlist_len > 0 {
                ui.playlist_selected = (ui.playlist_selected + 1).min(playlist_len - 1);
            }
        }
        UiAction::PlaylistRemove => {
            let now = rv_client.shared_now().await;
            let file_id = {
                let app = app_state.lock().await;
                let snapshot = app.sync_engine.state().playlist.snapshot();
                snapshot.get(ui.playlist_selected).copied()
            };
            if let Some(file_id) = file_id {
                let mut app = app_state.lock().await;
                app.process_event(AppEvent::RemoveFromPlaylist { file_id }, now);
            }
        }
        UiAction::PlaylistMoveUp => {
            let now = rv_client.shared_now().await;
            let mut app = app_state.lock().await;
            let snapshot = app.sync_engine.state().playlist.snapshot();
            if ui.playlist_selected > 0
                && let Some(file_id) = snapshot.get(ui.playlist_selected) {
                    let file_id = *file_id;
                    // Move before the item above: "after" the one two above, or None if moving to first
                    let after = if ui.playlist_selected >= 2 {
                        snapshot.get(ui.playlist_selected - 2).copied()
                    } else {
                        None
                    };
                    app.process_event(
                        AppEvent::MoveInPlaylist { file_id, after },
                        now,
                    );
                    ui.playlist_selected -= 1;
                }
        }
        UiAction::PlaylistMoveDown => {
            let now = rv_client.shared_now().await;
            let mut app = app_state.lock().await;
            let snapshot = app.sync_engine.state().playlist.snapshot();
            if ui.playlist_selected < snapshot.len().saturating_sub(1)
                && let Some(file_id) = snapshot.get(ui.playlist_selected) {
                    let file_id = *file_id;
                    let after = snapshot.get(ui.playlist_selected + 1).copied();
                    app.process_event(
                        AppEvent::MoveInPlaylist { file_id, after },
                        now,
                    );
                    ui.playlist_selected += 1;
                }
        }
        UiAction::OpenFileBrowser => {
            let media_roots = storage
                .lock()
                .ok()
                .and_then(|s| s.get_media_roots().ok())
                .unwrap_or_default();
            let start_dir = media_roots
                .first()
                .cloned()
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| PathBuf::from("/"));
            ui.file_browser = Some(FileBrowserState::open(start_dir, FileBrowserOrigin::Playlist));
            ui.screen = Screen::FileBrowser;
        }
        UiAction::ManualMapFile => {
            // Get the selected playlist item's FileId and metadata
            let info = {
                let app = app_state.lock().await;
                let crdt = app.sync_engine.state();
                let snapshot = crdt.playlist.snapshot();
                snapshot.get(ui.playlist_selected).map(|file_id| {
                    let filename = crdt.filenames.read(file_id).cloned();
                    let anime_id = crdt
                        .anidb
                        .read(file_id)
                        .and_then(|opt| opt.as_ref())
                        .map(|meta| meta.anime_id);
                    (*file_id, filename, anime_id)
                })
            };

            if let Some((file_id, filename, anime_id)) = info {
                let target_filename = filename.unwrap_or_default();

                // Smart default: determine starting directory
                // 1. If user previously mapped this series → that directory
                // 2. Otherwise → first media root or home
                let start_dir = anime_id
                    .and_then(|aid| {
                        storage
                            .lock()
                            .ok()
                            .and_then(|s| s.get_series_mapping_dir(aid).ok().flatten())
                    })
                    .or_else(|| {
                        storage
                            .lock()
                            .ok()
                            .and_then(|s| s.get_media_roots().ok())
                            .and_then(|roots| roots.into_iter().next())
                    })
                    .or_else(dirs::home_dir)
                    .unwrap_or_else(|| PathBuf::from("/"));

                ui.file_browser = Some(FileBrowserState::open(
                    start_dir,
                    FileBrowserOrigin::ManualMap {
                        file_id,
                        target_filename,
                    },
                ));
                ui.screen = Screen::FileBrowser;
            }
        }
        // Metadata assignment
        UiAction::AssignMetadata => {
            let app = app_state.lock().await;
            let crdt = app.sync_engine.state();
            let snapshot = crdt.playlist.snapshot();

            if let Some(file_id) = snapshot.get(ui.playlist_selected).copied() {
                // Collect unique series from all anidb entries
                let mut seen = std::collections::HashSet::new();
                let mut series_list = Vec::new();

                for (_key, (_ts, value)) in crdt.anidb.iter() {
                    if let Some(meta) = value
                        && seen.insert(meta.anime_id)
                    {
                        series_list.push(SeriesChoice {
                            anime_id: meta.anime_id,
                            name: meta.anime_name.clone(),
                        });
                    }
                }

                series_list.sort_by(|a, b| a.name.cmp(&b.name));
                drop(app);

                ui.metadata_assign = Some(MetadataAssignState {
                    file_id,
                    series_list,
                    selected: 0,
                    step: MetadataAssignStep::SelectSeries,
                    episode_input: InputState::new(),
                });
                ui.screen = Screen::MetadataAssign;
            }
        }
        UiAction::MetadataSelectUp => {
            if let Some(ref mut state) = ui.metadata_assign {
                state.selected = state.selected.saturating_sub(1);
            }
        }
        UiAction::MetadataSelectDown => {
            if let Some(ref mut state) = ui.metadata_assign
                && !state.series_list.is_empty()
            {
                state.selected = (state.selected + 1).min(state.series_list.len() - 1);
            }
        }
        UiAction::MetadataConfirmSeries => {
            if let Some(ref mut state) = ui.metadata_assign
                && !state.series_list.is_empty()
            {
                state.step = MetadataAssignStep::EnterEpisode;
            }
        }
        UiAction::MetadataInsertChar(c) => {
            if let Some(ref mut state) = ui.metadata_assign {
                state.episode_input.insert_char(*c);
            }
        }
        UiAction::MetadataDeleteBack => {
            if let Some(ref mut state) = ui.metadata_assign {
                state.episode_input.delete_back();
            }
        }
        UiAction::MetadataConfirmEpisode => {
            if let Some(ref state) = ui.metadata_assign {
                let episode = state.episode_input.text.trim().to_string();
                if !episode.is_empty()
                    && let Some(series) = state.series_list.get(state.selected)
                {
                    let file_id = state.file_id;
                    let anime_id = series.anime_id;
                    let anime_name = series.name.clone();

                    let now = rv_client.shared_now().await;
                    let mut app = app_state.lock().await;

                    // Determine MetadataSource based on current data
                    let current_source = app
                        .sync_engine
                        .state()
                        .anidb
                        .read(&file_id)
                        .and_then(|opt| opt.as_ref())
                        .map(|meta| meta.source);

                    let source = match current_source {
                        None | Some(MetadataSource::User) => MetadataSource::User,
                        Some(MetadataSource::AniDb)
                        | Some(MetadataSource::UserOverAniDb) => {
                            MetadataSource::UserOverAniDb
                        }
                    };

                    let metadata = AniDbMetadata {
                        anime_id,
                        anime_name,
                        episode_number: episode,
                        episode_name: String::new(),
                        group_name: String::new(),
                        source,
                    };

                    let op = CrdtOp::LwwWrite {
                        timestamp: now,
                        value: LwwValue::AniDb(file_id, Some(metadata)),
                    };
                    // Apply locally; sync will propagate via periodic summary
                    let _ = app.sync_engine.apply_local_op(op);
                }
            }
            ui.metadata_assign = None;
            ui.screen = Screen::Main;
        }
        UiAction::MetadataCancel => {
            ui.metadata_assign = None;
            ui.screen = Screen::Main;
        }
        // Recent series
        UiAction::RecentSelectUp => {
            ui.recent_selected = ui.recent_selected.saturating_sub(1);
        }
        UiAction::RecentSelectDown => {
            let app = app_state.lock().await;
            let crdt = app.sync_engine.state();
            let st = storage.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
            let count = series_browser::build_series_list(crdt, &st).len();
            drop(st);
            drop(app);
            if count > 0 {
                ui.recent_selected = (ui.recent_selected + 1).min(count - 1);
            }
        }
        UiAction::RecentSeriesSelect => {
            // Build series list, find the selected series, open file browser to its directory
            let info = {
                let app = app_state.lock().await;
                let crdt = app.sync_engine.state();
                let st = storage.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
                let list = series_browser::build_series_list(crdt, &st);

                list.get(ui.recent_selected).map(|entry| {
                    let dir = series_browser::series_directory(crdt, &st, entry.anime_id);
                    let next_file =
                        series_browser::next_unwatched_filename(crdt, &st, entry.anime_id);
                    (dir, next_file)
                })
            };

            if let Some((Some(dir), next_file)) = info {
                let mut fb = FileBrowserState::open(dir, FileBrowserOrigin::Playlist);

                // Position cursor on next unwatched episode if known
                if let Some(filename) = next_file
                    && let Some(idx) = fb.entries.iter().position(|e| e.name == filename)
                {
                    fb.selected = idx;
                }

                ui.file_browser = Some(fb);
                ui.screen = Screen::FileBrowser;
            }
        }
        // Settings
        UiAction::OpenSettings => {
            let (config, media_roots) = {
                let s = storage.lock().map_err(|e| anyhow::anyhow!("lock: {e}"))?;
                let config = s.get_config()?.ok_or_else(|| anyhow::anyhow!("no config"))?;
                let roots = s.get_media_roots().unwrap_or_default();
                (config, roots)
            };
            ui.settings = Some(SettingsState::from_config(&config, media_roots));
            ui.screen = Screen::Settings;
        }
        _ => {}
    }
    Ok(())
}

/// Dispatch AppEffects to the runtime.
#[allow(clippy::too_many_arguments)]
async fn dispatch_effects(
    effects: Vec<AppEffect>,
    peer_mgr: &Arc<PeerManager>,
    rv_client: &RendezvousClient,
    storage: &Arc<Mutex<ClientStorage>>,
    app_state: &Arc<tokio::sync::Mutex<AppState>>,
    gap_fill_tx: &tokio::sync::mpsc::UnboundedSender<(PeerId, GapFillResponse)>,
    player: &mut Option<MpvPlayer>,
    echo_filter: &mut EchoFilter,
    mtime_tracker: &mut MtimeTracker,
    rehash_tx: &tokio::sync::mpsc::UnboundedSender<(FileId, std::io::Result<FileId>)>,
) {
    for effect in effects {
        match effect {
            AppEffect::Sync(actions) => {
                dispatch_sync_actions(
                    actions, peer_mgr, rv_client, storage, app_state, gap_fill_tx,
                )
                .await;
            }
            AppEffect::Redraw => {} // Handled by needs_redraw flag
            AppEffect::PlayerPause => {
                if let Some(p) = player.as_ref() {
                    echo_filter.register_pause();
                    if let Err(e) = p.pause().await {
                        tracing::debug!("Failed to pause player: {e}");
                    }
                }
            }
            AppEffect::PlayerUnpause => {
                // Check file mtime before unpausing
                let current_file = app_state.lock().await.playback.current_file;
                let mut mtime_changed = false;
                if let Some(file_id) = current_file
                    && let Some(path) = mtime_tracker.check_mtime_changed(&file_id)
                {
                    tracing::info!(?file_id, "File mtime changed, re-hashing before unpause");
                    mtime_changed = true;
                    let tx = rehash_tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let result = (|| {
                            let file = std::fs::File::open(&path)?;
                            let reader = std::io::BufReader::new(file);
                            dessplay_core::ed2k::compute_ed2k(reader)
                        })();
                        let _ = tx.send((file_id, result));
                    });
                }
                if !mtime_changed
                    && let Some(p) = player.as_ref()
                {
                    echo_filter.register_unpause();
                    if let Err(e) = p.unpause().await {
                        tracing::debug!("Failed to unpause player: {e}");
                    }
                }
            }
            AppEffect::PlayerSeek(pos) => {
                if let Some(p) = player.as_ref() {
                    echo_filter.register_seek(pos);
                    if let Err(e) = p.seek(pos).await {
                        tracing::debug!("Failed to seek player: {e}");
                    }
                }
            }
            AppEffect::PlayerLoadFile(file_id) => {
                // Look up local file path from storage
                let local_path = storage
                    .lock()
                    .ok()
                    .and_then(|s| s.get_file_mapping(&file_id).ok().flatten());

                if let Some(path) = local_path {
                    // Ensure player is alive; launch if needed
                    let need_launch =
                        player.as_ref().is_none_or(|p| !p.is_alive());
                    if need_launch {
                        match MpvPlayer::launch().await {
                            Ok(p) => {
                                *player = Some(p);
                                tracing::info!("Launched mpv player");
                            }
                            Err(e) => {
                                tracing::warn!("Failed to launch mpv: {e}");
                                continue;
                            }
                        }
                    }
                    if let Some(p) = player.as_ref()
                        && let Err(e) = p.load_file(&path).await
                    {
                        tracing::warn!("Failed to load file into player: {e}");
                    }
                    // Record file mtime for re-hash on unpause
                    mtime_tracker.record_mtime(file_id, &path);
                } else {
                    tracing::debug!(?file_id, "No local path for file, skipping load");
                }
            }
            AppEffect::PlayerShowOsd(text) => {
                if let Some(p) = player.as_ref()
                    && let Err(e) = p.show_osd(&text, 3000).await
                {
                    tracing::debug!("Failed to show OSD: {e}");
                }
            }
            AppEffect::MarkWatched(file_id) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as u64);
                if let Ok(s) = storage.lock()
                    && let Err(e) = s.mark_watched(&file_id, now)
                {
                    tracing::warn!("Failed to mark file as watched: {e}");
                }
            }
        }
    }
}

/// Dispatch sync actions to network/storage.
async fn dispatch_sync_actions(
    actions: Vec<SyncAction>,
    peer_mgr: &Arc<PeerManager>,
    rv_client: &RendezvousClient,
    storage: &Arc<Mutex<ClientStorage>>,
    app_state: &Arc<tokio::sync::Mutex<AppState>>,
    gap_fill_tx: &tokio::sync::mpsc::UnboundedSender<(PeerId, GapFillResponse)>,
) {
    for action in actions {
        match action {
            SyncAction::SendControl { peer, msg } => {
                if peer == PeerId(0) {
                    let rv_msg = match msg {
                        PeerControl::StateOp { op } => RvControl::StateOp { op },
                        PeerControl::StateSummary { versions, .. } => {
                            RvControl::StateSummary { versions }
                        }
                        PeerControl::StateSnapshot { epoch, crdts } => {
                            RvControl::StateSnapshot { epoch, crdts }
                        }
                        _ => continue,
                    };
                    rv_client.send(rv_msg);
                } else if let Err(e) = peer_mgr.send_control(peer, &msg).await {
                    tracing::debug!(%peer, "Failed to send control: {e}");
                }
            }
            SyncAction::SendDatagram { peer, msg } => {
                if peer != PeerId(0) {
                    let _ = peer_mgr.send_datagram(peer, &msg).await;
                }
            }
            SyncAction::BroadcastControl { msg } => {
                for peer in peer_mgr.connected_peers().await {
                    let _ = peer_mgr.send_control(peer, &msg).await;
                }
                let rv_msg = match &msg {
                    PeerControl::StateOp { op } => {
                        Some(RvControl::StateOp { op: op.clone() })
                    }
                    PeerControl::StateSummary { versions, .. } => {
                        Some(RvControl::StateSummary {
                            versions: versions.clone(),
                        })
                    }
                    _ => None,
                };
                if let Some(rv_msg) = rv_msg {
                    rv_client.send(rv_msg);
                }
            }
            SyncAction::BroadcastDatagram { msg } => {
                for peer in peer_mgr.connected_peers().await {
                    let _ = peer_mgr.send_datagram(peer, &msg).await;
                }
            }
            SyncAction::RequestGapFill { peer, request } => {
                if peer == PeerId(0) {
                    tracing::debug!("Gap fill to server not supported via streams");
                } else {
                    let peer_mgr = Arc::clone(peer_mgr);
                    let tx = gap_fill_tx.clone();
                    tokio::spawn(async move {
                        match do_gap_fill(peer, request, &peer_mgr).await {
                            Ok(response) => {
                                let _ = tx.send((peer, response));
                            }
                            Err(e) => {
                                tracing::debug!(%peer, "Gap fill request failed: {e}");
                            }
                        }
                    });
                }
            }
            SyncAction::PersistOp { op } => {
                let epoch = app_state.lock().await.sync_engine.epoch();
                if let Ok(s) = storage.lock()
                    && let Err(e) = s.append_op(epoch, &op)
                {
                    tracing::warn!("Failed to persist op: {e}");
                }
            }
            SyncAction::PersistSnapshot { epoch, snapshot } => {
                if let Ok(s) = storage.lock()
                    && let Err(e) = s.save_snapshot(epoch, &snapshot)
                {
                    tracing::warn!("Failed to persist snapshot: {e}");
                }
            }
        }
    }
}

async fn do_gap_fill(
    peer: PeerId,
    request: GapFillRequest,
    peer_mgr: &PeerManager,
) -> Result<GapFillResponse> {
    let mut stream = peer_mgr.open_stream(peer).await?;
    write_framed(&mut stream.send, TAG_GAP_FILL_REQUEST, &request).await?;
    let response: GapFillResponse = read_framed(&mut stream.recv, TAG_GAP_FILL_RESPONSE)
        .await?
        .ok_or_else(|| anyhow::anyhow!("gap fill stream closed without response"))?;
    Ok(response)
}

async fn handle_incoming_stream(
    mut stream: dessplay_core::network::MessageStream,
    app_state: Arc<tokio::sync::Mutex<AppState>>,
) -> Result<()> {
    let request: GapFillRequest = read_framed(&mut stream.recv, TAG_GAP_FILL_REQUEST)
        .await?
        .ok_or_else(|| anyhow::anyhow!("stream closed before gap fill request"))?;

    let response = {
        let app = app_state.lock().await;
        app.on_gap_fill_request(&request)
    };
    write_framed(&mut stream.send, TAG_GAP_FILL_RESPONSE, &response).await?;
    Ok(())
}

/// Try auto-matching all unmapped playlist items against the media index.
/// Also evaluates known/unknown series for files that remain unmatched,
/// setting FileState::Missing and (for unknown series) UserState::NotWatching.
async fn try_auto_match_all(
    app_state: &Arc<tokio::sync::Mutex<AppState>>,
    media_index: &MediaIndex,
    storage: &Arc<Mutex<ClientStorage>>,
    now: SharedTimestamp,
) -> Vec<AppEffect> {
    // Collect watched file IDs from storage
    let watched_files: Vec<FileId> = storage
        .lock()
        .ok()
        .and_then(|s| s.watched_files().ok())
        .unwrap_or_default()
        .into_iter()
        .map(|(fid, _ts)| fid)
        .collect();

    // Phase 1: Read CRDT state under lock, collect per-file info
    let file_info = {
        let app = app_state.lock().await;
        let crdt = app.sync_engine.state();
        let playlist = crdt.playlist.snapshot();
        let our_user_id = &app.our_user_id;

        // Build set of anime_ids from watched files
        let mut known_anime_ids = std::collections::HashSet::new();
        for fid in &watched_files {
            if let Some(Some(meta)) = crdt.anidb.read(fid) {
                known_anime_ids.insert(meta.anime_id);
            }
        }

        playlist
            .iter()
            .map(|file_id| {
                let already_mapped = storage
                    .lock()
                    .ok()
                    .and_then(|s| s.get_file_mapping(file_id).ok().flatten())
                    .is_some();
                let filename = crdt.filenames.read(file_id).cloned();
                let is_known_series = crdt
                    .anidb
                    .read(file_id)
                    .and_then(|opt| opt.as_ref())
                    .is_some_and(|meta| known_anime_ids.contains(&meta.anime_id));
                let current_file_state = crdt
                    .file_states
                    .read(&(our_user_id.clone(), *file_id))
                    .cloned();
                (*file_id, already_mapped, filename, is_known_series, current_file_state)
            })
            .collect::<Vec<_>>()
    };

    if file_info.is_empty() {
        return vec![];
    }

    let current_file_id = file_info.first().map(|(fid, ..)| *fid);

    // Phase 2: Try auto-matching, collect events for state changes
    let mut events: Vec<AppEvent> = Vec::new();
    let mut placeholder_filename: Option<String> = None;

    for (file_id, already_mapped, filename, is_known_series, current_file_state) in &file_info {
        if *already_mapped {
            continue;
        }

        let matched = filename
            .as_deref()
            .is_some_and(|name| try_auto_match_file(file_id, name, media_index, storage));

        if matched {
            // Just matched — clear Missing state if set
            if matches!(current_file_state, Some(FileState::Missing)) {
                events.push(AppEvent::SetFileState {
                    file_id: *file_id,
                    state: FileState::Ready,
                });
            }
        } else {
            // Not matched — set Missing if not already
            if !matches!(current_file_state, Some(FileState::Missing)) {
                events.push(AppEvent::SetFileState {
                    file_id: *file_id,
                    state: FileState::Missing,
                });
            }

            // Unknown series + current file → set NotWatching + show OSD placeholder
            if !is_known_series && current_file_id == Some(*file_id) {
                events.push(AppEvent::SetUserState {
                    state: UserState::NotWatching,
                });
                placeholder_filename = filename.clone();
            }
        }
    }

    // Phase 3: Process collected events
    if events.is_empty() {
        return vec![];
    }

    let mut app = app_state.lock().await;
    let mut all_effects = Vec::new();
    for event in events {
        all_effects.extend(app.process_event(event, now));
    }

    // Show OSD placeholder when set to NotWatching
    if let Some(filename) = placeholder_filename {
        all_effects.push(AppEffect::PlayerShowOsd(format!(
            "You don't have this file:\n{filename}"
        )));
    }

    all_effects
}

/// Try to auto-match a single file by filename against the media index.
/// Returns true if a mapping was created.
fn try_auto_match_file(
    file_id: &FileId,
    filename: &str,
    media_index: &MediaIndex,
    storage: &Arc<Mutex<ClientStorage>>,
) -> bool {
    if let Some(paths) = media_index.find_by_filename(filename) {
        // Prefer the first match (from the first media root)
        if let Some(path) = paths.first()
            && let Ok(s) = storage.lock()
        {
            if let Err(e) = s.set_file_mapping(file_id, path) {
                tracing::warn!(?file_id, "Failed to store auto-match: {e}");
                return false;
            }
            tracing::info!(?file_id, path = %path.display(), "Auto-matched file");
            return true;
        }
    }
    false
}

fn get_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod mtime_tests {
    use super::*;
    use dessplay_core::types::FileId;
    use std::io::Write;

    fn fid(n: u8) -> FileId {
        let mut arr = [0u8; 16];
        arr[0] = n;
        FileId(arr)
    }

    #[test]
    fn record_and_check_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.mkv");
        std::fs::write(&path, b"hello").unwrap();

        let mut tracker = MtimeTracker::new();
        tracker.record_mtime(fid(1), &path);

        // Mtime unchanged — should return None
        assert!(tracker.check_mtime_changed(&fid(1)).is_none());
    }

    #[test]
    fn detect_mtime_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.mkv");
        std::fs::write(&path, b"hello").unwrap();

        let mut tracker = MtimeTracker::new();
        tracker.record_mtime(fid(1), &path);

        // Modify the file (change mtime)
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        file.write_all(b"world").unwrap();
        file.flush().unwrap();
        drop(file);

        // Mtime changed — should return Some(path)
        let result = tracker.check_mtime_changed(&fid(1));
        assert!(result.is_some());
        assert_eq!(result.unwrap(), path);
    }

    #[test]
    fn manually_mapped_skips_check() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.mkv");
        std::fs::write(&path, b"hello").unwrap();

        let mut tracker = MtimeTracker::new();
        tracker.record_mtime(fid(1), &path);
        tracker.manually_mapped.insert(fid(1));

        // Modify the file
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&path, b"world").unwrap();

        // Should return None because manually mapped
        assert!(tracker.check_mtime_changed(&fid(1)).is_none());
    }

    #[test]
    fn unknown_file_returns_none() {
        let tracker = MtimeTracker::new();
        assert!(tracker.check_mtime_changed(&fid(99)).is_none());
    }

    #[test]
    fn record_mtime_updates_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.mkv");
        std::fs::write(&path, b"hello").unwrap();

        let mut tracker = MtimeTracker::new();
        tracker.record_mtime(fid(1), &path);

        // Modify the file
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&path, b"world").unwrap();

        // Re-record mtime (simulating what happens after successful re-hash)
        tracker.record_mtime(fid(1), &path);

        // Should now be considered unchanged
        assert!(tracker.check_mtime_changed(&fid(1)).is_none());
    }

    #[test]
    fn deleted_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.mkv");
        std::fs::write(&path, b"hello").unwrap();

        let mut tracker = MtimeTracker::new();
        tracker.record_mtime(fid(1), &path);

        // Delete the file
        std::fs::remove_file(&path).unwrap();

        // Should return None (can't stat deleted file)
        assert!(tracker.check_mtime_changed(&fid(1)).is_none());
    }
}
