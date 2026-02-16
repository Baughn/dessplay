mod layout;
mod panes;

use std::io;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use futures::StreamExt;
use tokio::sync::mpsc;

pub struct ChatMessage {
    pub timestamp: String,
    pub text: String,
    pub min_verbosity: u8,
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
    chat_messages: Vec<ChatMessage>,
    connected_peers: Vec<String>,
    focused: FocusedPane,
    verbosity: u8,
    username: String,
    should_quit: bool,
}

impl App {
    pub fn new(username: String, verbosity: u8) -> Self {
        Self {
            chat_messages: Vec::new(),
            connected_peers: Vec::new(),
            focused: FocusedPane::Chat,
            verbosity,
            username,
            should_quit: false,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Tab => {
                self.focused = self.focused.next();
            }
            _ => {}
        }
    }

    fn push_message(&mut self, text: String, min_verbosity: u8) {
        let now = chrono_now();
        self.chat_messages.push(ChatMessage {
            timestamp: now,
            text,
            min_verbosity,
        });
    }

    fn render(&self, frame: &mut ratatui::Frame) {
        let rects = layout::compute_layout(frame.area());

        panes::chat::render(frame, rects.chat, &self.chat_messages, self.verbosity);
        panes::chat::render_input(frame, rects.chat_input);
        panes::series::render(frame, rects.recent_series);
        panes::users::render(frame, rects.users, &self.username, &self.connected_peers);
        panes::playlist::render(frame, rects.playlist);
        panes::status::render(frame, rects.player_status);
    }
}

/// Messages sent to the TUI event loop from background tasks.
pub enum AppEvent {
    PeerConnected(String),
    PeerDisconnected(String),
    ConnectionStateChanged { peer: String, state: String },
    SystemMessage { text: String, min_verbosity: u8 },
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
                        app.push_message(format!("{peer} connected"), 0);
                        if !app.connected_peers.contains(&peer) {
                            app.connected_peers.push(peer);
                        }
                    }
                    AppEvent::PeerDisconnected(peer) => {
                        app.push_message(format!("{peer} disconnected"), 0);
                        app.connected_peers.retain(|p| p != &peer);
                    }
                    AppEvent::ConnectionStateChanged { peer, state } => {
                        app.push_message(
                            format!("{peer}: {state}"),
                            1,
                        );
                    }
                    AppEvent::SystemMessage { text, min_verbosity } => {
                        app.push_message(text, min_verbosity);
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
