mod input;
mod layout;
mod panes;

use std::sync::Arc;

use std::io;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::network::sync::LocalEvent;
use crate::state::SharedState;

use self::input::TextInput;

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
}

impl App {
    pub fn new(username: String, verbosity: u8) -> Self {
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
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Ctrl-C always quits
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }

        match self.focused {
            FocusedPane::Chat => self.handle_chat_key(key),
            FocusedPane::RecentSeries | FocusedPane::Playlist => {
                if key.code == KeyCode::Tab {
                    self.focused = self.focused.next();
                }
            }
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
        match text.trim() {
            "/exit" | "/quit" | "/q" => self.should_quit = true,
            _ => self.push_system_message(format!("Unknown command: {text}"), 0),
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
        let rects = layout::compute_layout(frame.area());

        let messages = self.build_chat_messages();
        panes::chat::render(frame, rects.chat, &messages, self.verbosity);
        panes::chat::render_input(
            frame,
            rects.chat_input,
            self.input.text(),
            self.input.cursor_pos(),
            self.focused == FocusedPane::Chat,
        );
        panes::series::render(frame, rects.recent_series);
        panes::users::render(frame, rects.users, &self.username, &self.connected_peers);
        panes::playlist::render(frame, rects.playlist);
        panes::status::render(frame, rects.player_status);
        panes::keybindings::render(frame, rects.keybindings, self.focused);
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

    #[test]
    fn handle_command_exit() {
        let mut app = App::new("test".into(), 0);
        app.handle_command("/exit");
        assert!(app.should_quit);
    }

    #[test]
    fn handle_command_quit() {
        let mut app = App::new("test".into(), 0);
        app.handle_command("/quit");
        assert!(app.should_quit);
    }

    #[test]
    fn handle_command_q() {
        let mut app = App::new("test".into(), 0);
        app.handle_command("/q");
        assert!(app.should_quit);
    }

    #[test]
    fn handle_command_unknown() {
        let mut app = App::new("test".into(), 0);
        app.handle_command("/foo");
        assert!(!app.should_quit);
        assert_eq!(app.system_messages.len(), 1);
        assert!(app.system_messages[0].text.contains("Unknown command"));
    }
}
