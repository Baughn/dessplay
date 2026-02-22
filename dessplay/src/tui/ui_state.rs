use std::path::PathBuf;

use crate::app_state::AppEvent;

/// Which pane has focus in the main screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusedPane {
    Chat,
    RecentSeries,
    Playlist,
}

impl FocusedPane {
    pub fn next(self) -> Self {
        match self {
            Self::Chat => Self::RecentSeries,
            Self::RecentSeries => Self::Playlist,
            Self::Playlist => Self::Chat,
        }
    }
}

/// Which screen is currently displayed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Main,
    Settings,
    FileBrowser,
    TofuWarning,
}

/// Text input state with char-boundary-aware cursor.
#[derive(Clone, Debug, Default)]
pub struct InputState {
    pub text: String,
    /// Cursor position in chars (not bytes).
    pub cursor: usize,
}

impl InputState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a character at the cursor position.
    pub fn insert_char(&mut self, c: char) {
        let byte_pos = self.char_to_byte(self.cursor);
        self.text.insert(byte_pos, c);
        self.cursor += 1;
    }

    /// Delete the character before the cursor (backspace).
    pub fn delete_back(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let byte_pos = self.char_to_byte(self.cursor);
            self.text.remove(byte_pos);
        }
    }

    /// Delete the character after the cursor (delete key).
    pub fn delete_forward(&mut self) {
        let char_count = self.text.chars().count();
        if self.cursor < char_count {
            let byte_pos = self.char_to_byte(self.cursor);
            self.text.remove(byte_pos);
        }
    }

    /// Move cursor one char left.
    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move cursor one char right.
    pub fn move_right(&mut self) {
        let char_count = self.text.chars().count();
        if self.cursor < char_count {
            self.cursor += 1;
        }
    }

    /// Move cursor one word left.
    pub fn move_word_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let chars: Vec<char> = self.text.chars().collect();
        let mut pos = self.cursor - 1;
        // Skip whitespace
        while pos > 0 && chars[pos].is_whitespace() {
            pos -= 1;
        }
        // Skip word chars
        while pos > 0 && !chars[pos - 1].is_whitespace() {
            pos -= 1;
        }
        self.cursor = pos;
    }

    /// Move cursor one word right.
    pub fn move_word_right(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let len = chars.len();
        if self.cursor >= len {
            return;
        }
        let mut pos = self.cursor;
        // Skip word chars
        while pos < len && !chars[pos].is_whitespace() {
            pos += 1;
        }
        // Skip whitespace
        while pos < len && chars[pos].is_whitespace() {
            pos += 1;
        }
        self.cursor = pos;
    }

    /// Move cursor to start.
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to end.
    pub fn move_end(&mut self) {
        self.cursor = self.text.chars().count();
    }

    /// Clear the input.
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Take the text and reset.
    pub fn take_text(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    /// Convert char index to byte index.
    fn char_to_byte(&self, char_idx: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_idx)
            .map(|(i, _)| i)
            .unwrap_or(self.text.len())
    }

    /// Number of chars in the text.
    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }
}

/// State for the settings screen.
#[derive(Clone, Debug)]
pub struct SettingsState {
    pub username: String,
    pub server: String,
    pub player: String,
    pub password: String,
    pub media_roots: Vec<PathBuf>,
    /// Which field has focus (0=username, 1=server, 2=player, 3=password, 4=media_roots).
    pub focused_field: usize,
    pub field_count: usize,
    /// Error banner shown at the top of the settings screen.
    pub alert: Option<String>,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsState {
    pub fn new() -> Self {
        let username = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default();
        Self {
            username,
            server: "dessplay.brage.info:4433".to_string(),
            player: "mpv".to_string(),
            password: String::new(),
            media_roots: Vec::new(),
            focused_field: 0,
            field_count: 5,
            alert: None,
        }
    }

    pub fn from_config(config: &crate::storage::Config, media_roots: Vec<PathBuf>) -> Self {
        Self {
            username: config.username.clone(),
            server: config.server.clone(),
            player: config.player.clone(),
            password: config.password.clone().unwrap_or_default(),
            media_roots,
            focused_field: 0,
            field_count: 5,
            alert: None,
        }
    }

    pub fn next_field(&mut self) {
        self.focused_field = (self.focused_field + 1) % self.field_count;
    }

    pub fn prev_field(&mut self) {
        if self.focused_field == 0 {
            self.focused_field = self.field_count - 1;
        } else {
            self.focused_field -= 1;
        }
    }

    /// Returns true if the config is valid (non-empty username, valid server, >=1 media root).
    pub fn is_valid(&self) -> bool {
        self.is_username_valid() && self.is_server_valid() && self.has_media_roots()
    }

    pub fn is_username_valid(&self) -> bool {
        !self.username.trim().is_empty()
    }

    pub fn is_server_valid(&self) -> bool {
        validate_server_format(&self.server).is_ok()
    }

    pub fn server_error(&self) -> Option<&'static str> {
        validate_server_format(&self.server).err()
    }

    pub fn has_media_roots(&self) -> bool {
        !self.media_roots.is_empty()
    }
}

/// Validate that a server string is in `host:port` format with a valid port.
/// Returns `Ok(())` on valid, `Err(reason)` on invalid.
pub fn validate_server_format(server: &str) -> Result<(), &'static str> {
    let server = server.trim();
    if server.is_empty() {
        return Err("server required");
    }

    // Handle IPv6 bracket notation: [::1]:4433
    let (host, port_str) = if server.starts_with('[') {
        // IPv6 literal
        let bracket_end = server.find(']').ok_or("missing closing ']' for IPv6 address")?;
        let host = &server[1..bracket_end];
        if host.is_empty() {
            return Err("empty host");
        }
        let rest = &server[bracket_end + 1..];
        let port_str = rest.strip_prefix(':').ok_or("expected ':port' after IPv6 address")?;
        (host, port_str)
    } else {
        // hostname or IPv4: split on last ':'
        let colon = server.rfind(':').ok_or("expected host:port")?;
        let host = &server[..colon];
        if host.is_empty() {
            return Err("empty host");
        }
        let port_str = &server[colon + 1..];
        (host, port_str)
    };

    // Validate host is non-empty (already checked above, but be safe)
    let _ = host;

    // Validate port
    if port_str.is_empty() {
        return Err("expected host:port");
    }
    let port: u32 = port_str.parse().map_err(|_| "port must be a number")?;
    if port == 0 || port > 65535 {
        return Err("port must be 1-65535");
    }

    Ok(())
}

/// State for the file browser.
#[derive(Clone, Debug)]
pub struct FileBrowserState {
    pub current_dir: PathBuf,
    pub entries: Vec<FileBrowserEntry>,
    pub selected: usize,
    pub scroll_offset: usize,
    /// Where to return to after file browser closes.
    pub origin: FileBrowserOrigin,
}

#[derive(Clone, Debug)]
pub struct FileBrowserEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileBrowserOrigin {
    /// Adding a file to the playlist.
    Playlist,
    /// Adding a media root from settings.
    SettingsMediaRoot,
}

impl FileBrowserState {
    pub fn open(dir: PathBuf, origin: FileBrowserOrigin) -> Self {
        let mut state = Self {
            current_dir: dir,
            entries: Vec::new(),
            selected: 0,
            scroll_offset: 0,
            origin,
        };
        state.refresh_entries();
        state
    }

    /// Re-read the directory contents.
    pub fn refresh_entries(&mut self) {
        let mut dirs = Vec::new();
        let mut files = Vec::new();

        if let Ok(read_dir) = std::fs::read_dir(&self.current_dir) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip hidden files
                if name.starts_with('.') {
                    continue;
                }
                if path.is_dir() {
                    dirs.push(FileBrowserEntry {
                        name,
                        path,
                        is_dir: true,
                    });
                } else if is_media_file(&name) || self.origin == FileBrowserOrigin::SettingsMediaRoot
                {
                    files.push(FileBrowserEntry {
                        name,
                        path,
                        is_dir: false,
                    });
                }
            }
        }

        dirs.sort_by(|a, b| a.name.cmp(&b.name));
        files.sort_by(|a, b| a.name.cmp(&b.name));

        self.entries = Vec::new();
        // Parent directory entry
        if let Some(parent) = self.current_dir.parent() {
            self.entries.push(FileBrowserEntry {
                name: "..".to_string(),
                path: parent.to_path_buf(),
                is_dir: true,
            });
        }
        self.entries.extend(dirs);
        // Only show files when browsing for playlist files, not for media root dirs
        if self.origin != FileBrowserOrigin::SettingsMediaRoot {
            self.entries.extend(files);
        }

        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
    }

    pub fn select_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn select_down(&mut self) {
        if !self.entries.is_empty() {
            self.selected = (self.selected + 1).min(self.entries.len() - 1);
        }
    }
}

/// Check if a filename looks like a media file.
fn is_media_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".mkv")
        || lower.ends_with(".mp4")
        || lower.ends_with(".avi")
        || lower.ends_with(".webm")
        || lower.ends_with(".m4v")
        || lower.ends_with(".mov")
        || lower.ends_with(".wmv")
        || lower.ends_with(".flv")
        || lower.ends_with(".ogm")
        || lower.ends_with(".ts")
}

/// State for the TOFU certificate mismatch warning modal.
#[derive(Clone, Debug)]
pub struct TofuWarningState {
    pub server: String,
    pub stored_fingerprint: Vec<u8>,
    pub received_fingerprint: Vec<u8>,
}

/// The complete TUI state, separate from the app state.
pub struct UiState {
    pub screen: Screen,
    pub focus: FocusedPane,
    pub input: InputState,
    pub chat_scroll: usize,
    pub playlist_selected: usize,
    pub recent_selected: usize,
    pub settings: Option<SettingsState>,
    pub file_browser: Option<FileBrowserState>,
    pub tofu_warning: Option<TofuWarningState>,
    pub should_quit: bool,
    /// Status message shown temporarily.
    pub status_message: Option<String>,
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

impl UiState {
    pub fn new() -> Self {
        Self {
            screen: Screen::Main,
            focus: FocusedPane::Chat,
            input: InputState::new(),
            chat_scroll: 0,
            playlist_selected: 0,
            recent_selected: 0,
            settings: None,
            file_browser: None,
            tofu_warning: None,
            should_quit: false,
            status_message: None,
        }
    }

    /// Start with settings screen (first run).
    pub fn with_settings(mut self) -> Self {
        self.screen = Screen::Settings;
        self.settings = Some(SettingsState::new());
        self
    }
}

/// Result of processing an input event.
pub enum InputResult {
    /// Produces an AppEvent to send to AppState.
    AppEvent(AppEvent),
    /// Changes only the UiState (no AppState mutation).
    UiAction(UiAction),
    /// Both an AppEvent and a UiAction.
    Both(AppEvent, UiAction),
    /// Nothing happened.
    None,
}

/// Actions that only affect the UI state.
#[derive(Debug)]
pub enum UiAction {
    Quit,
    CycleFocus,
    // Chat
    InsertChar(char),
    DeleteBack,
    DeleteForward,
    CursorLeft,
    CursorRight,
    CursorWordLeft,
    CursorWordRight,
    CursorHome,
    CursorEnd,
    ClearInput,
    ScrollChatUp,
    ScrollChatDown,
    // Playlist
    PlaylistSelectUp,
    PlaylistSelectDown,
    PlaylistMoveUp,
    PlaylistMoveDown,
    PlaylistRemove,
    OpenFileBrowser,
    // Recent series
    RecentSelectUp,
    RecentSelectDown,
    // Settings
    SettingsNextField,
    SettingsPrevField,
    SettingsSave,
    SettingsInsertChar(char),
    SettingsDeleteBack,
    SettingsTogglePlayer,
    SettingsAddMediaRoot,
    SettingsRemoveMediaRoot,
    SettingsMoveRootUp,
    SettingsMoveRootDown,
    SettingsCancel,
    // Settings (from main screen)
    OpenSettings,
    // File browser
    FileBrowserUp,
    FileBrowserDown,
    FileBrowserSelect,
    FileBrowserBack,
    FileBrowserSelectDir,
    // TOFU warning
    TofuAccept,
    TofuReject,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn input_insert_and_take() {
        let mut input = InputState::new();
        input.insert_char('h');
        input.insert_char('i');
        assert_eq!(input.text, "hi");
        assert_eq!(input.cursor, 2);
        let text = input.take_text();
        assert_eq!(text, "hi");
        assert_eq!(input.text, "");
        assert_eq!(input.cursor, 0);
    }

    #[test]
    fn input_delete_back() {
        let mut input = InputState::new();
        input.insert_char('a');
        input.insert_char('b');
        input.insert_char('c');
        input.delete_back();
        assert_eq!(input.text, "ab");
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn input_delete_back_at_start() {
        let mut input = InputState::new();
        input.delete_back(); // Should be no-op
        assert_eq!(input.text, "");
        assert_eq!(input.cursor, 0);
    }

    #[test]
    fn input_delete_forward() {
        let mut input = InputState::new();
        input.insert_char('a');
        input.insert_char('b');
        input.insert_char('c');
        input.move_home();
        input.delete_forward();
        assert_eq!(input.text, "bc");
        assert_eq!(input.cursor, 0);
    }

    #[test]
    fn input_delete_forward_at_end() {
        let mut input = InputState::new();
        input.insert_char('a');
        input.delete_forward(); // Should be no-op
        assert_eq!(input.text, "a");
    }

    #[test]
    fn input_cursor_movement() {
        let mut input = InputState::new();
        input.insert_char('h');
        input.insert_char('e');
        input.insert_char('l');
        input.insert_char('l');
        input.insert_char('o');

        input.move_left();
        assert_eq!(input.cursor, 4);
        input.move_left();
        assert_eq!(input.cursor, 3);
        input.move_right();
        assert_eq!(input.cursor, 4);
        input.move_home();
        assert_eq!(input.cursor, 0);
        input.move_end();
        assert_eq!(input.cursor, 5);
    }

    #[test]
    fn input_word_movement() {
        let mut input = InputState::new();
        for c in "hello world foo".chars() {
            input.insert_char(c);
        }
        // cursor at end (15)
        input.move_word_left();
        assert_eq!(input.cursor, 12); // start of "foo"
        input.move_word_left();
        assert_eq!(input.cursor, 6); // start of "world"
        input.move_word_left();
        assert_eq!(input.cursor, 0); // start of "hello"
        input.move_word_left();
        assert_eq!(input.cursor, 0); // stays at 0

        input.move_word_right();
        assert_eq!(input.cursor, 6); // after "hello "
        input.move_word_right();
        assert_eq!(input.cursor, 12); // after "world "
        input.move_word_right();
        assert_eq!(input.cursor, 15); // end
    }

    #[test]
    fn input_unicode() {
        let mut input = InputState::new();
        for c in "こんにちは".chars() {
            input.insert_char(c);
        }
        assert_eq!(input.char_count(), 5);
        assert_eq!(input.cursor, 5);
        input.move_left();
        assert_eq!(input.cursor, 4);
        input.delete_back();
        assert_eq!(input.text, "こんには");
        assert_eq!(input.cursor, 3);
    }

    #[test]
    fn input_insert_in_middle() {
        let mut input = InputState::new();
        input.insert_char('a');
        input.insert_char('c');
        input.move_left();
        input.insert_char('b');
        assert_eq!(input.text, "abc");
        assert_eq!(input.cursor, 2);
    }

    #[test]
    fn input_clear() {
        let mut input = InputState::new();
        input.insert_char('x');
        input.clear();
        assert_eq!(input.text, "");
        assert_eq!(input.cursor, 0);
    }

    #[test]
    fn focus_cycling() {
        assert_eq!(FocusedPane::Chat.next(), FocusedPane::RecentSeries);
        assert_eq!(FocusedPane::RecentSeries.next(), FocusedPane::Playlist);
        assert_eq!(FocusedPane::Playlist.next(), FocusedPane::Chat);
    }

    #[test]
    fn settings_validation() {
        let mut s = SettingsState::new();
        s.username = "alice".into();
        s.server = "example.com:4433".into();
        assert!(!s.is_valid()); // no media roots
        s.media_roots.push(PathBuf::from("/anime"));
        assert!(s.is_valid());
        s.username.clear();
        assert!(!s.is_valid()); // empty username
    }

    #[test]
    fn validate_server_format_valid() {
        use super::validate_server_format;
        assert!(validate_server_format("localhost:4433").is_ok());
        assert!(validate_server_format("dessplay.brage.info:4433").is_ok());
        assert!(validate_server_format("192.168.1.1:8080").is_ok());
        assert!(validate_server_format("[::1]:4433").is_ok());
        assert!(validate_server_format("example.com:1").is_ok());
        assert!(validate_server_format("example.com:65535").is_ok());
    }

    #[test]
    fn validate_server_format_invalid() {
        use super::validate_server_format;
        assert!(validate_server_format("").is_err());
        assert!(validate_server_format("localhost").is_err()); // no port
        assert!(validate_server_format("localhost:0").is_err()); // port 0
        assert!(validate_server_format("localhost:65536").is_err()); // port too high
        assert!(validate_server_format("localhost:abc").is_err()); // non-numeric port
        assert!(validate_server_format(":4433").is_err()); // no host
        assert!(validate_server_format("localhost:").is_err()); // empty port
    }

    #[test]
    fn settings_validation_rejects_bad_server() {
        let mut s = SettingsState::new();
        s.username = "alice".into();
        s.server = "localhost".into(); // no port
        s.media_roots.push(PathBuf::from("/anime"));
        assert!(!s.is_valid());
        assert!(!s.is_server_valid());
        assert!(s.server_error().is_some());

        s.server = "localhost:4433".into();
        assert!(s.is_valid());
        assert!(s.is_server_valid());
        assert!(s.server_error().is_none());
    }

    #[test]
    fn settings_field_cycling() {
        let mut s = SettingsState::new();
        assert_eq!(s.focused_field, 0);
        s.next_field();
        assert_eq!(s.focused_field, 1);
        s.next_field();
        s.next_field();
        s.next_field();
        s.next_field();
        assert_eq!(s.focused_field, 0); // wraps around
        s.prev_field();
        assert_eq!(s.focused_field, 4);
    }
}
