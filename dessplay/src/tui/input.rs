use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app_state::AppEvent;
use crate::tui::ui_state::{FocusedPane, InputResult, Screen, UiAction, UiState};

/// Map a crossterm KeyEvent to an InputResult based on the current UI state.
pub fn handle_key_event(key: KeyEvent, ui: &UiState) -> InputResult {
    // Ctrl-C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return InputResult::UiAction(UiAction::Quit);
    }

    match &ui.screen {
        Screen::Settings => handle_settings_key(key, ui),
        Screen::FileBrowser => handle_file_browser_key(key),
        Screen::TofuWarning => handle_tofu_warning_key(key),
        Screen::Main => handle_main_key(key, ui),
        Screen::Hashing => InputResult::None, // only Ctrl-C (handled above) during hashing
        Screen::MetadataAssign => handle_metadata_assign_key(key, ui),
    }
}

fn handle_main_key(key: KeyEvent, ui: &UiState) -> InputResult {
    // Tab cycles focus in all panes
    if key.code == KeyCode::Tab && !key.modifiers.contains(KeyModifiers::SHIFT) {
        return InputResult::UiAction(UiAction::CycleFocus);
    }

    // Ctrl-S opens settings (must be before pane dispatch so chat doesn't swallow it)
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        return InputResult::UiAction(UiAction::OpenSettings);
    }

    match ui.focus {
        FocusedPane::Chat => handle_chat_key(key, ui),
        FocusedPane::Playlist => handle_playlist_key(key),
        FocusedPane::RecentSeries => handle_recent_series_key(key),
    }
}

fn handle_chat_key(key: KeyEvent, ui: &UiState) -> InputResult {
    match key.code {
        KeyCode::Enter => {
            let text = ui.input.text.trim().to_string();
            if text.is_empty() {
                return InputResult::None;
            }
            // Check for /commands
            match text.as_str() {
                "/exit" | "/quit" | "/q" => {
                    return InputResult::UiAction(UiAction::Quit);
                }
                _ => {}
            }
            InputResult::Both(
                AppEvent::SendChat { text },
                UiAction::ClearInput,
            )
        }
        KeyCode::Esc => InputResult::UiAction(UiAction::ClearInput),
        KeyCode::Backspace => InputResult::UiAction(UiAction::DeleteBack),
        KeyCode::Delete => InputResult::UiAction(UiAction::DeleteForward),
        KeyCode::Left => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                InputResult::UiAction(UiAction::CursorWordLeft)
            } else {
                InputResult::UiAction(UiAction::CursorLeft)
            }
        }
        KeyCode::Right => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                InputResult::UiAction(UiAction::CursorWordRight)
            } else {
                InputResult::UiAction(UiAction::CursorRight)
            }
        }
        KeyCode::Home => InputResult::UiAction(UiAction::CursorHome),
        KeyCode::End => InputResult::UiAction(UiAction::CursorEnd),
        KeyCode::Up => InputResult::UiAction(UiAction::ScrollChatUp),
        KeyCode::Down => InputResult::UiAction(UiAction::ScrollChatDown),
        KeyCode::Char(c) => InputResult::UiAction(UiAction::InsertChar(c)),
        _ => InputResult::None,
    }
}

fn handle_playlist_key(key: KeyEvent) -> InputResult {
    // Check Ctrl modifiers first, before bare characters
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('a') => InputResult::UiAction(UiAction::AssignMetadata),
            KeyCode::Char('m') => InputResult::UiAction(UiAction::ManualMapFile),
            KeyCode::Char('j') => InputResult::UiAction(UiAction::PlaylistMoveDown),
            KeyCode::Char('k') => InputResult::UiAction(UiAction::PlaylistMoveUp),
            _ => InputResult::None,
        };
    }
    match key.code {
        KeyCode::Up => InputResult::UiAction(UiAction::PlaylistSelectUp),
        KeyCode::Down => InputResult::UiAction(UiAction::PlaylistSelectDown),
        KeyCode::Char('a') => InputResult::UiAction(UiAction::OpenFileBrowser),
        KeyCode::Char('d') => InputResult::UiAction(UiAction::PlaylistRemove),
        _ => InputResult::None,
    }
}

fn handle_recent_series_key(key: KeyEvent) -> InputResult {
    match key.code {
        KeyCode::Up => InputResult::UiAction(UiAction::RecentSelectUp),
        KeyCode::Down => InputResult::UiAction(UiAction::RecentSelectDown),
        KeyCode::Enter => InputResult::UiAction(UiAction::RecentSeriesSelect),
        _ => InputResult::None,
    }
}

fn handle_settings_key(key: KeyEvent, ui: &UiState) -> InputResult {
    let settings = match &ui.settings {
        Some(s) => s,
        None => return InputResult::None,
    };

    // Ctrl-S saves
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        return InputResult::UiAction(UiAction::SettingsSave);
    }

    match key.code {
        KeyCode::Esc => InputResult::UiAction(UiAction::SettingsCancel),
        KeyCode::Tab => {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                InputResult::UiAction(UiAction::SettingsPrevField)
            } else {
                InputResult::UiAction(UiAction::SettingsNextField)
            }
        }
        KeyCode::Up => InputResult::UiAction(UiAction::SettingsPrevField),
        KeyCode::Down => InputResult::UiAction(UiAction::SettingsNextField),
        KeyCode::Enter => {
            match settings.focused_field {
                2 => InputResult::UiAction(UiAction::SettingsTogglePlayer), // player toggle
                4 => InputResult::UiAction(UiAction::SettingsAddMediaRoot), // media roots: add
                _ => InputResult::None,
            }
        }
        KeyCode::Char('d') if settings.focused_field == 4 => {
            InputResult::UiAction(UiAction::SettingsRemoveMediaRoot)
        }
        KeyCode::Char('j')
            if settings.focused_field == 4 && key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            InputResult::UiAction(UiAction::SettingsMoveRootDown)
        }
        KeyCode::Char('k')
            if settings.focused_field == 4 && key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            InputResult::UiAction(UiAction::SettingsMoveRootUp)
        }
        KeyCode::Backspace => {
            // Only text fields: username(0), server(1), password(3)
            match settings.focused_field {
                0 | 1 | 3 => InputResult::UiAction(UiAction::SettingsDeleteBack),
                _ => InputResult::None,
            }
        }
        KeyCode::Char(c) => {
            match settings.focused_field {
                0 | 1 | 3 => InputResult::UiAction(UiAction::SettingsInsertChar(c)),
                _ => InputResult::None,
            }
        }
        _ => InputResult::None,
    }
}

fn handle_tofu_warning_key(key: KeyEvent) -> InputResult {
    match key.code {
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            InputResult::UiAction(UiAction::TofuAccept)
        }
        KeyCode::Esc => InputResult::UiAction(UiAction::TofuReject),
        _ => InputResult::None,
    }
}

fn handle_metadata_assign_key(key: KeyEvent, ui: &UiState) -> InputResult {
    let Some(ref state) = ui.metadata_assign else {
        return InputResult::None;
    };

    match state.step {
        crate::tui::ui_state::MetadataAssignStep::SelectSeries => match key.code {
            KeyCode::Up => InputResult::UiAction(UiAction::MetadataSelectUp),
            KeyCode::Down => InputResult::UiAction(UiAction::MetadataSelectDown),
            KeyCode::Enter => InputResult::UiAction(UiAction::MetadataConfirmSeries),
            KeyCode::Esc => InputResult::UiAction(UiAction::MetadataCancel),
            _ => InputResult::None,
        },
        crate::tui::ui_state::MetadataAssignStep::EnterEpisode => match key.code {
            KeyCode::Enter => InputResult::UiAction(UiAction::MetadataConfirmEpisode),
            KeyCode::Esc => InputResult::UiAction(UiAction::MetadataCancel),
            KeyCode::Backspace => InputResult::UiAction(UiAction::MetadataDeleteBack),
            KeyCode::Char(c) => InputResult::UiAction(UiAction::MetadataInsertChar(c)),
            _ => InputResult::None,
        },
    }
}

fn handle_file_browser_key(key: KeyEvent) -> InputResult {
    match key.code {
        KeyCode::Up => InputResult::UiAction(UiAction::FileBrowserUp),
        KeyCode::Down => InputResult::UiAction(UiAction::FileBrowserDown),
        KeyCode::Enter => InputResult::UiAction(UiAction::FileBrowserSelect),
        KeyCode::Esc => InputResult::UiAction(UiAction::FileBrowserBack),
        // In SettingsMediaRoot mode, 's' selects current directory
        KeyCode::Char('s') => InputResult::UiAction(UiAction::FileBrowserSelectDir),
        _ => InputResult::None,
    }
}
