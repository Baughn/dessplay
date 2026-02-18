mod input;
mod layout;
mod modal;
mod panes;
pub mod series_state;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use std::io;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::network::sync::LocalEvent;
use crate::network::PeerId;
use crate::state::types::{ItemId, PlaylistAction};
use crate::state::SharedState;
use crate::storage::Database;

use self::input::TextInput;
use self::modal::FileBrowserModal;
use self::series_state::SeriesPaneState;

/// A message displayed in the chat pane (system messages + synced chat).
pub struct DisplayMessage {
    pub timestamp: String,
    pub text: String,
    pub min_verbosity: u8,
    /// Monotonic counter for merge-sorting system and synced messages.
    sort_key: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FocusedPane {
    Chat,
    RecentSeries,
    Playlist,
}

impl FocusedPane {
    fn next(self) -> Self {
        match self {
            Self::Chat => Self::RecentSeries,
            Self::RecentSeries => Self::Playlist,
            Self::Playlist => Self::Chat,
        }
    }
}

/// File resolution status for playlist items.
#[derive(Clone)]
pub enum FileResolution {
    Resolved(PathBuf),
    Missing,
}

pub struct App {
    system_messages: Vec<DisplayMessage>,
    connected_peers: Vec<String>,
    focused: FocusedPane,
    verbosity: u8,
    username: String,
    should_quit: bool,
    input: TextInput,
    message_counter: u64,
    /// Shared state from sync engine (set once SyncReady arrives).
    shared_state: Option<Arc<SharedState>>,
    /// Channel to send local events to the sync engine.
    local_event_tx: Option<mpsc::UnboundedSender<LocalEvent>>,
    /// How many synced chat messages we've already assigned sort keys to.
    synced_chat_seen: usize,
    /// Sort keys assigned to synced messages (parallel to shared_state chat_messages).
    synced_sort_keys: Vec<u64>,
    /// Local persistence.
    db: Arc<Database>,
    /// Recent Series pane state.
    series_pane: SeriesPaneState,
    /// Active file browser modal, if any.
    modal: Option<FileBrowserModal>,
    /// Maps playlist filenames to their local resolution.
    file_resolutions: HashMap<String, FileResolution>,
    /// Monotonic counter for playlist item IDs (per this user).
    playlist_seq: u64,
    /// Sender to update the resolved file path for the player control loop.
    resolved_path_tx: Option<tokio::sync::watch::Sender<Option<PathBuf>>>,
    /// Last current_file ItemId we sent a resolved path for.
    last_resolved_file: Option<ItemId>,
}

impl App {
    pub fn new(username: String, verbosity: u8, db: Arc<Database>) -> Self {
        Self {
            system_messages: Vec::new(),
            connected_peers: Vec::new(),
            focused: FocusedPane::Chat,
            verbosity,
            username,
            should_quit: false,
            input: TextInput::new(),
            message_counter: 0,
            shared_state: None,
            local_event_tx: None,
            synced_chat_seen: 0,
            synced_sort_keys: Vec::new(),
            db,
            series_pane: SeriesPaneState::new(),
            modal: None,
            file_resolutions: HashMap::new(),
            playlist_seq: 0,
            resolved_path_tx: None,
            last_resolved_file: None,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Ctrl-C always quits
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }

        // Modal intercepts all input when open
        if self.modal.is_some() {
            self.handle_modal_key(key);
            return;
        }

        match self.focused {
            FocusedPane::Chat => self.handle_chat_key(key),
            FocusedPane::RecentSeries => self.handle_series_key(key),
            FocusedPane::Playlist => {
                if key.code == KeyCode::Tab {
                    self.focused = self.focused.next();
                }
            }
        }
    }

    fn handle_modal_key(&mut self, key: KeyEvent) {
        // Safety: caller checked self.modal.is_some()
        match key.code {
            KeyCode::Esc => {
                self.modal = None;
            }
            KeyCode::Up => {
                if let Some(modal) = &mut self.modal {
                    modal.move_up();
                }
            }
            KeyCode::Down => {
                if let Some(modal) = &mut self.modal {
                    modal.move_down();
                }
            }
            KeyCode::Backspace => {
                if let Some(modal) = &mut self.modal {
                    modal.go_up(&self.db);
                }
            }
            KeyCode::Enter => {
                let result = self.modal.as_mut().and_then(|m| m.enter(&self.db));
                if let Some(selected) = result {
                    // Pre-populate resolution so we don't need to re-scan
                    self.file_resolutions.insert(
                        selected.filename.clone(),
                        FileResolution::Resolved(selected.full_path),
                    );
                    self.add_file_to_playlist(selected.filename);
                    self.modal = None;
                }
            }
            _ => {}
        }
    }

    fn handle_series_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab => self.focused = self.focused.next(),
            KeyCode::Up => self.series_pane.move_up(),
            KeyCode::Down => self.series_pane.move_down(),
            KeyCode::Enter => {
                if let Some(item) = self.series_pane.selected_item() {
                    let browse_path = item.browse_path();
                    if browse_path.is_dir() {
                        self.modal = Some(FileBrowserModal::new(browse_path, &self.db));
                    } else {
                        self.push_system_message(
                            format!("Directory not found: {}", browse_path.display()),
                            0,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_chat_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Tab => {
                self.focused = self.focused.next();
            }
            KeyCode::Enter => {
                let text = self.input.take();
                if !text.is_empty() {
                    self.submit_chat(text);
                }
            }
            KeyCode::Esc => {
                self.input.clear();
            }
            KeyCode::Backspace => {
                self.input.backspace();
            }
            KeyCode::Delete => {
                self.input.delete();
            }
            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.move_word_left();
            }
            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.move_word_right();
            }
            KeyCode::Left => {
                self.input.move_left();
            }
            KeyCode::Right => {
                self.input.move_right();
            }
            KeyCode::Home => {
                self.input.home();
            }
            KeyCode::End => {
                self.input.end();
            }
            KeyCode::Char(c) => {
                self.input.insert(c);
            }
            _ => {}
        }
    }

    fn submit_chat(&mut self, text: String) {
        if text.starts_with('/') {
            self.handle_command(&text);
        } else if let Some(tx) = &self.local_event_tx {
            let _ = tx.send(LocalEvent::ChatSent { text });
        } else {
            self.push_system_message("Not connected to sync engine yet".into(), 0);
        }
    }

    fn handle_command(&mut self, text: &str) {
        let parts: Vec<&str> = text.trim().splitn(2, ' ').collect();
        match parts[0] {
            "/exit" | "/quit" | "/q" => self.should_quit = true,
            "/add-root" => {
                if let Some(path) = parts.get(1) {
                    let path = path.trim();
                    if !std::path::Path::new(path).is_dir() {
                        self.push_system_message(
                            format!("Not a directory: {path}"),
                            0,
                        );
                        return;
                    }
                    match self.db.add_media_root(path) {
                        Ok(()) => {
                            self.push_system_message(format!("Added media root: {path}"), 0);
                            self.series_pane.dirty = true;
                        }
                        Err(e) => {
                            self.push_system_message(format!("Failed to add root: {e}"), 0);
                        }
                    }
                } else {
                    self.push_system_message("Usage: /add-root <path>".into(), 0);
                }
            }
            "/remove-root" => {
                if let Some(path) = parts.get(1) {
                    let path = path.trim();
                    match self.db.remove_media_root(path) {
                        Ok(true) => {
                            self.push_system_message(format!("Removed media root: {path}"), 0);
                            self.series_pane.dirty = true;
                        }
                        Ok(false) => {
                            self.push_system_message(format!("Not found: {path}"), 0);
                        }
                        Err(e) => {
                            self.push_system_message(format!("Failed to remove root: {e}"), 0);
                        }
                    }
                } else {
                    self.push_system_message("Usage: /remove-root <path>".into(), 0);
                }
            }
            "/list-roots" => match self.db.list_media_roots() {
                Ok(roots) if roots.is_empty() => {
                    self.push_system_message(
                        "No media roots configured. Use /add-root <path>".into(),
                        0,
                    );
                }
                Ok(roots) => {
                    self.push_system_message("Media roots:".into(), 0);
                    for (i, root) in roots.iter().enumerate() {
                        self.push_system_message(format!("  {}. {}", i + 1, root), 0);
                    }
                }
                Err(e) => {
                    self.push_system_message(format!("Error: {e}"), 0);
                }
            },
            _ => self.push_system_message(format!("Unknown command: {text}"), 0),
        }
    }

    fn add_file_to_playlist(&mut self, filename: String) {
        let Some(tx) = self.local_event_tx.clone() else {
            self.push_system_message("Not connected to sync engine yet".into(), 0);
            return;
        };

        // Place after last item in playlist
        let after = self.shared_state.as_ref().and_then(|state| {
            let view = state.view();
            view.playlist.last().map(|item| item.id.clone())
        });

        self.playlist_seq += 1;
        let id = ItemId {
            user: PeerId(self.username.clone()),
            seq: self.playlist_seq,
        };

        self.push_system_message(format!("Added to playlist: {filename}"), 0);

        let _ = tx.send(LocalEvent::PlaylistAction(PlaylistAction::Add {
            id,
            filename,
            after,
        }));
    }

    /// Initialize playlist_seq from existing playlist state (on SyncReady).
    fn init_playlist_seq(&mut self) {
        if let Some(state) = &self.shared_state {
            let view = state.view();
            let my_peer = PeerId(self.username.clone());
            let max_seq = view
                .playlist
                .iter()
                .filter(|item| item.id.user == my_peer)
                .map(|item| item.id.seq)
                .max()
                .unwrap_or(0);
            self.playlist_seq = max_seq;
        }
    }

    /// Resolve any new playlist items against media roots.
    fn resolve_playlist_files(&mut self) {
        let view = match &self.shared_state {
            Some(s) => s.view(),
            None => return,
        };

        let roots = match self.db.list_media_roots() {
            Ok(r) => r,
            Err(_) => return,
        };

        for item in &view.playlist {
            if !self.file_resolutions.contains_key(&item.filename) {
                match crate::files::scanner::find_file(&roots, &item.filename) {
                    Some(path) => {
                        self.file_resolutions
                            .insert(item.filename.clone(), FileResolution::Resolved(path));
                    }
                    None => {
                        self.file_resolutions
                            .insert(item.filename.clone(), FileResolution::Missing);
                    }
                }
            }
        }

        // Update the resolved path for the player if current file changed
        self.update_resolved_path(&view);
    }

    /// Send the resolved path for the current file to the player control loop.
    fn update_resolved_path(&mut self, view: &crate::state::StateView) {
        if view.current_file == self.last_resolved_file {
            return;
        }
        self.last_resolved_file.clone_from(&view.current_file);

        if let Some(tx) = &self.resolved_path_tx {
            let path = view.current_file.as_ref().and_then(|file_id| {
                // Find the filename for this ItemId
                let filename = view.playlist.iter().find(|i| i.id == *file_id)?;
                match self.file_resolutions.get(&filename.filename)? {
                    FileResolution::Resolved(p) => Some(p.clone()),
                    FileResolution::Missing => None,
                }
            });
            let _ = tx.send(path);
        }
    }

    fn next_sort_key(&mut self) -> u64 {
        self.message_counter += 1;
        self.message_counter
    }

    fn push_system_message(&mut self, text: String, min_verbosity: u8) {
        let sort_key = self.next_sort_key();
        let now = chrono_now();
        self.system_messages.push(DisplayMessage {
            timestamp: now,
            text,
            min_verbosity,
            sort_key,
        });
    }

    /// Build the merged, sorted message list for display.
    fn build_chat_messages(&mut self) -> Vec<DisplayMessage> {
        // Assign sort keys to any new synced messages
        if let Some(state) = &self.shared_state {
            let view = state.view();
            let chat = &view.chat_messages;
            while self.synced_chat_seen < chat.len() {
                let key = self.next_sort_key();
                self.synced_sort_keys.push(key);
                self.synced_chat_seen += 1;
            }
        }

        let mut messages: Vec<DisplayMessage> = Vec::new();

        // Add system messages
        for msg in &self.system_messages {
            messages.push(DisplayMessage {
                timestamp: msg.timestamp.clone(),
                text: msg.text.clone(),
                min_verbosity: msg.min_verbosity,
                sort_key: msg.sort_key,
            });
        }

        // Add synced chat messages
        if let Some(state) = &self.shared_state {
            let view = state.view();
            for (i, msg) in view.chat_messages.iter().enumerate() {
                if let Some(&sort_key) = self.synced_sort_keys.get(i) {
                    messages.push(DisplayMessage {
                        timestamp: format_shared_timestamp(msg.timestamp),
                        text: format!("<{}> {}", msg.sender.0, msg.text),
                        min_verbosity: 0,
                        sort_key,
                    });
                }
            }
        }

        messages.sort_by_key(|m| m.sort_key);
        messages
    }

    fn render(&mut self, frame: &mut ratatui::Frame) {
        // Refresh series pane if dirty
        if self.series_pane.dirty {
            self.series_pane.refresh(&self.db);
        }

        // Resolve playlist files on each render tick
        self.resolve_playlist_files();

        let rects = layout::compute_layout(frame.area());

        let view = self.shared_state.as_ref().map(|s| s.view());

        let messages = self.build_chat_messages();
        panes::chat::render(frame, rects.chat, &messages, self.verbosity);
        panes::chat::render_input(
            frame,
            rects.chat_input,
            self.input.text(),
            self.input.cursor_pos(),
            self.focused == FocusedPane::Chat,
        );
        panes::series::render(
            frame,
            rects.recent_series,
            &self.series_pane,
            self.focused == FocusedPane::RecentSeries,
        );
        panes::users::render(frame, rects.users, &self.username, &self.connected_peers);
        panes::playlist::render(
            frame,
            rects.playlist,
            view.as_ref(),
            &self.file_resolutions,
        );
        panes::status::render(frame, rects.player_status, view.as_ref());
        panes::keybindings::render(
            frame,
            rects.keybindings,
            self.focused,
            self.modal.is_some(),
        );

        // Render modal on top if open
        if let Some(modal) = &mut self.modal {
            modal::render(frame, modal);
        }
    }
}

/// Messages sent to the TUI event loop from background tasks.
pub enum AppEvent {
    PeerConnected(String),
    PeerDisconnected(String),
    ConnectionStateChanged {
        peer: String,
        state: String,
    },
    SystemMessage {
        text: String,
        min_verbosity: u8,
    },
    SyncReady {
        state: Arc<SharedState>,
        event_tx: mpsc::UnboundedSender<LocalEvent>,
    },
    /// Sender for the resolved file path channel (player control loop reads this).
    ResolvedPathSender(tokio::sync::watch::Sender<Option<PathBuf>>),
}

/// Run the TUI event loop. Returns when the user quits.
pub async fn run(mut app: App, mut event_rx: mpsc::UnboundedReceiver<AppEvent>) -> io::Result<()> {
    let mut terminal = ratatui::init();

    let mut event_stream = crossterm::event::EventStream::new();
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));

    loop {
        terminal.draw(|frame| app.render(frame))?;

        tokio::select! {
            Some(event) = event_stream.next() => {
                if let Ok(Event::Key(key)) = event {
                    app.handle_key(key);
                }
            }
            Some(app_event) = event_rx.recv() => {
                match app_event {
                    AppEvent::PeerConnected(peer) => {
                        app.push_system_message(format!("{peer} connected"), 0);
                        if !app.connected_peers.contains(&peer) {
                            app.connected_peers.push(peer);
                        }
                    }
                    AppEvent::PeerDisconnected(peer) => {
                        app.push_system_message(format!("{peer} disconnected"), 0);
                        app.connected_peers.retain(|p| p != &peer);
                    }
                    AppEvent::ConnectionStateChanged { peer, state } => {
                        app.push_system_message(
                            format!("{peer}: {state}"),
                            1,
                        );
                    }
                    AppEvent::SystemMessage { text, min_verbosity } => {
                        app.push_system_message(text, min_verbosity);
                    }
                    AppEvent::SyncReady { state, event_tx } => {
                        app.shared_state = Some(state);
                        app.local_event_tx = Some(event_tx);
                        app.init_playlist_seq();
                    }
                    AppEvent::ResolvedPathSender(tx) => {
                        app.resolved_path_tx = Some(tx);
                    }
                }
            }
            _ = tick.tick() => {
                // Redraw on tick (handles resize etc.)
            }
        }

        if app.should_quit {
            break;
        }
    }

    ratatui::restore();
    Ok(())
}

fn chrono_now() -> String {
    // Format as HH:MM:SS using std time
    let now = std::time::SystemTime::now();
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

fn format_shared_timestamp(ts: crate::network::clock::SharedTimestamp) -> String {
    // SharedTimestamp is microseconds; convert to HH:MM:SS
    let total_secs = (ts.0 / 1_000_000).unsigned_abs();
    let h = (total_secs / 3600) % 24;
    let m = (total_secs / 60) % 60;
    let s = total_secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        let db = Arc::new(Database::open_in_memory().unwrap());
        App::new("test".into(), 0, db)
    }

    #[test]
    fn handle_command_exit() {
        let mut app = test_app();
        app.handle_command("/exit");
        assert!(app.should_quit);
    }

    #[test]
    fn handle_command_quit() {
        let mut app = test_app();
        app.handle_command("/quit");
        assert!(app.should_quit);
    }

    #[test]
    fn handle_command_q() {
        let mut app = test_app();
        app.handle_command("/q");
        assert!(app.should_quit);
    }

    #[test]
    fn handle_command_unknown() {
        let mut app = test_app();
        app.handle_command("/foo");
        assert!(!app.should_quit);
        assert_eq!(app.system_messages.len(), 1);
        assert!(app.system_messages[0].text.contains("Unknown command"));
    }

    #[test]
    fn handle_add_root_valid() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = test_app();
        app.handle_command(&format!("/add-root {}", dir.path().display()));
        // Should have a success message
        assert!(app.system_messages.iter().any(|m| m.text.contains("Added media root")));
        assert!(app.series_pane.dirty);
    }

    #[test]
    fn handle_add_root_nonexistent() {
        let mut app = test_app();
        app.handle_command("/add-root /nonexistent/path/that/does/not/exist");
        assert!(app.system_messages.iter().any(|m| m.text.contains("Not a directory")));
    }

    #[test]
    fn handle_add_root_missing_arg() {
        let mut app = test_app();
        app.handle_command("/add-root");
        assert!(app.system_messages.iter().any(|m| m.text.contains("Usage:")));
    }

    #[test]
    fn handle_list_roots_empty() {
        let mut app = test_app();
        app.handle_command("/list-roots");
        assert!(app.system_messages.iter().any(|m| m.text.contains("No media roots")));
    }

    #[test]
    fn handle_remove_root() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = test_app();
        let path = dir.path().to_str().unwrap();
        app.handle_command(&format!("/add-root {path}"));
        app.handle_command(&format!("/remove-root {path}"));
        assert!(app.system_messages.iter().any(|m| m.text.contains("Removed media root")));
    }
}
