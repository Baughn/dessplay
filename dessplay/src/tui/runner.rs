use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::EventStream;
use tokio_stream::StreamExt;

use crate::app_state::{AppEffect, AppEvent, AppState};
use crate::peer_conn::PeerManager;
use crate::rendezvous_client::{RendezvousClient, RendezvousEvent};
use crate::storage::{ClientStorage, Config};
use crate::tui::layout::compute_layout;
use crate::tui::terminal::{setup_file_logging, setup_terminal};
use crate::tui::ui_state::{
    FileBrowserOrigin, FileBrowserState, FocusedPane, InputResult, Screen, UiAction, UiState,
};
use crate::tui::widgets::{
    chat, file_browser, keybinding_bar, player_status, playlist, recent_series, settings, users,
};
use dessplay_core::framing::{
    read_framed, write_framed, TAG_GAP_FILL_REQUEST, TAG_GAP_FILL_RESPONSE,
};
use dessplay_core::network::NetworkEvent;
use dessplay_core::protocol::{
    GapFillRequest, GapFillResponse, PeerControl, PeerDatagram, RvControl,
};
use dessplay_core::sync_engine::{SyncAction, SyncEngine};
use dessplay_core::types::{FileState, PeerId, UserId, UserState};

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
    }

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
    run_connected(&mut guard.terminal, &mut ui, &storage, &config, args).await
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
                    s.media_roots.pop();
                }
        }
        UiAction::SettingsMoveRootUp => {
            // For simplicity, swap last two roots (full reorder logic in future)
            if let Some(ref mut s) = ui.settings {
                let len = s.media_roots.len();
                if len >= 2 {
                    s.media_roots.swap(len - 2, len - 1);
                }
            }
        }
        UiAction::SettingsMoveRootDown => {
            if let Some(ref mut s) = ui.settings {
                let len = s.media_roots.len();
                if len >= 2 {
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
) -> Result<()> {
    // Get password
    let password = config
        .password
        .clone()
        .or_else(|| std::env::var("DESSPLAY_PASSWORD").ok())
        .context("no password configured")?;

    let server_str = get_arg(args, "--server").unwrap_or_else(|| config.server.clone());

    let server_addr: std::net::SocketAddr = server_str
        .parse()
        .context("invalid server address — expected host:port")?;

    let tofu = Arc::new(crate::tls::TofuVerifier::new(
        Arc::clone(storage),
        server_str.clone(),
    ));

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

    // Send initial state summary
    {
        let app = app_state.lock().await;
        rv_client.send(RvControl::StateSummary {
            versions: app.sync_engine.version_vectors(),
        });
    }

    let (gap_fill_tx, mut gap_fill_rx) =
        tokio::sync::mpsc::unbounded_channel::<(PeerId, GapFillResponse)>();

    let mut summary_interval = tokio::time::interval(Duration::from_secs(1));
    let mut event_stream = EventStream::new();
    let mut needs_redraw = true;

    loop {
        // Draw if needed
        if needs_redraw {
            draw_main_screen(terminal, ui, &app_state, storage).await?;
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
                                ui.file_browser = None;
                                ui.screen = Screen::Main;

                                let path_for_hash = path.clone();
                                let hash_result = tokio::task::spawn_blocking(move || {
                                    let file = std::fs::File::open(&path_for_hash)?;
                                    let reader = std::io::BufReader::new(file);
                                    dessplay_core::ed2k::compute_ed2k(reader)
                                }).await;

                                match hash_result {
                                    Ok(Ok(file_id)) => {
                                        if let Ok(s) = storage.lock() {
                                            let _ = s.set_file_mapping(&file_id, &path);
                                        }
                                        let now = rv_client.shared_now().await;
                                        let effects = app_state.lock().await.process_event(
                                            AppEvent::AddToPlaylist { file_id, after: None },
                                            now,
                                        );
                                        dispatch_effects(
                                            effects, &peer_mgr, &rv_client, storage,
                                            &app_state, &gap_fill_tx,
                                        ).await;
                                    }
                                    Ok(Err(e)) => {
                                        tracing::warn!("Failed to hash file: {e}");
                                        ui.status_message = Some(format!("Hash error: {e}"));
                                    }
                                    Err(e) => {
                                        tracing::warn!("Hash task panicked: {e}");
                                    }
                                }
                            } else if ui.file_browser.is_none() {
                                // File browser was closed without selection
                                ui.screen = Screen::Main;
                            }

                            needs_redraw = true;
                            continue;
                        }

                        let result = crate::tui::input::handle_key_event(key, ui);
                        match result {
                            InputResult::AppEvent(event) => {
                                let now = rv_client.shared_now().await;
                                let effects = app_state.lock().await.process_event(event, now);
                                dispatch_effects(
                                    effects, &peer_mgr, &rv_client, storage,
                                    &app_state, &gap_fill_tx,
                                ).await;
                                needs_redraw = true;
                            }
                            InputResult::UiAction(action) => {
                                apply_main_ui_action(ui, &action, storage, &app_state, &rv_client).await?;
                                needs_redraw = true;
                            }
                            InputResult::Both(event, action) => {
                                let now = rv_client.shared_now().await;
                                let effects = app_state.lock().await.process_event(event, now);
                                dispatch_effects(
                                    effects, &peer_mgr, &rv_client, storage,
                                    &app_state, &gap_fill_tx,
                                ).await;
                                apply_main_ui_action(ui, &action, storage, &app_state, &rv_client).await?;
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
                        let effects = app_state.lock().await.process_event(
                            AppEvent::RemoteOp { from: PeerId(0), op },
                            now,
                        );
                        dispatch_effects(
                            effects, &peer_mgr, &rv_client, storage,
                            &app_state, &gap_fill_tx,
                        ).await;
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
                        ).await;
                    }
                    Ok(NetworkEvent::PeerControl { from, message }) => {
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
                            ).await;
                        }
                    }
                    Ok(NetworkEvent::PeerDatagram { from, message }) => {
                        let app_event = match message {
                            PeerDatagram::StateOp { op } => {
                                Some(AppEvent::RemoteOp { from, op })
                            }
                            _ => None,
                        };
                        if let Some(event) = app_event {
                            let effects = app_state.lock().await.process_event(event, now);
                            dispatch_effects(
                                effects, &peer_mgr, &rv_client, storage,
                                &app_state, &gap_fill_tx,
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
                    ).await;
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
                ).await;
                // Don't redraw on every tick unless effects requested it
            }
        }
    }

    Ok(())
}

/// Draw the main screen.
async fn draw_main_screen(
    terminal: &mut crate::tui::terminal::Tui,
    ui: &UiState,
    app_state: &Arc<tokio::sync::Mutex<AppState>>,
    storage: &Arc<Mutex<ClientStorage>>,
) -> Result<()> {
    // Collect all data from app state into owned values, then drop the lock
    let (chat_msgs, user_entries, playlist_entries, current_file_name) = {
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
                let display_name =
                    playlist::file_display_name(file_id, local_path.as_deref());
                let is_missing = local_path.is_none();
                playlist::PlaylistEntry {
                    file_id: *file_id,
                    display_name,
                    is_missing,
                    is_current: i == 0,
                }
            })
            .collect();

        let current_file_name: Option<String> =
            playlist_entries.first().map(|e| e.display_name.clone());

        (chat_msgs, user_entries, playlist_entries, current_file_name)
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

        // Recent series (stub)
        recent_series::render_recent_series(
            layout.recent_series,
            frame.buffer_mut(),
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

        // Player status (stub)
        player_status::render_player_status(
            layout.player_status,
            frame.buffer_mut(),
            current_file_name.as_deref(),
        );

        // Keybinding bar
        keybinding_bar::render_keybinding_bar(
            layout.keybinding_bar,
            frame.buffer_mut(),
            &ui.focus,
        );
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
        // Recent series
        UiAction::RecentSelectUp => {
            ui.recent_selected = ui.recent_selected.saturating_sub(1);
        }
        UiAction::RecentSelectDown => {
            ui.recent_selected += 1; // Stub: no bounds check until Phase 8
        }
        _ => {}
    }
    Ok(())
}

/// Dispatch AppEffects to the runtime.
async fn dispatch_effects(
    effects: Vec<AppEffect>,
    peer_mgr: &Arc<PeerManager>,
    rv_client: &RendezvousClient,
    storage: &Arc<Mutex<ClientStorage>>,
    app_state: &Arc<tokio::sync::Mutex<AppState>>,
    gap_fill_tx: &tokio::sync::mpsc::UnboundedSender<(PeerId, GapFillResponse)>,
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
            AppEffect::PlayerPause
            | AppEffect::PlayerUnpause
            | AppEffect::PlayerSeek(_)
            | AppEffect::PlayerLoadFile(_)
            | AppEffect::PlayerShowOsd(_) => {} // Phase 7
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

fn get_arg(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}
