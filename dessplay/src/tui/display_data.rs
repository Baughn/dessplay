//! Pre-computed display data extracted from AppState + ClientStorage.
//!
//! `build_display_data()` collects all data needed by `view()` into an owned
//! struct.  This allows `view()` to be a pure function with no storage or lock
//! dependencies.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use dessplay_core::types::{FileId, FileState, UserId, UserState};

use crate::app_state::AppState;
use crate::series_browser::{self, SeriesEntry};
use crate::storage::ClientStorage;
use crate::tui::runner::BgHashProgress;

// ---------------------------------------------------------------------------
// Display entry types
// ---------------------------------------------------------------------------

/// A user row for the Users pane.
#[derive(Debug, Clone)]
pub struct UserDisplayEntry {
    pub name: String,
    pub user_state: UserState,
    pub file_state: FileState,
    pub is_self: bool,
}

/// A playlist row for the Playlist pane.
#[derive(Debug, Clone)]
pub struct PlaylistDisplayEntry {
    pub file_id: FileId,
    pub display_name: String,
    pub is_missing: bool,
    pub is_current: bool,
}

// ---------------------------------------------------------------------------
// DisplayData
// ---------------------------------------------------------------------------

/// All data needed by `view()` to build a `ViewSpec`, pre-extracted from
/// app state and storage so the view function is pure.
#[derive(Debug, Clone)]
pub struct DisplayData {
    pub chat_messages: Vec<(UserId, String)>,
    pub user_entries: Vec<UserDisplayEntry>,
    pub playlist_entries: Vec<PlaylistDisplayEntry>,
    pub series_entries: Vec<SeriesEntry>,
    pub current_file_name: Option<String>,
    pub position_secs: f64,
    pub duration_secs: Option<f64>,
    pub is_playing: bool,
    pub blocking_users: Vec<String>,
    pub bg_hash_progress: Option<BgHashDisplayData>,
}

/// Pre-computed display data for background indexing progress.
#[derive(Debug, Clone)]
pub struct BgHashDisplayData {
    pub completed_files: u64,
    pub total_files: u64,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    pub rate_bps: Option<f64>,
    pub eta_secs: Option<f64>,
}

impl DisplayData {
    /// Empty display data for pre-connection screens (no app state available).
    pub fn empty() -> Self {
        Self {
            chat_messages: Vec::new(),
            user_entries: Vec::new(),
            playlist_entries: Vec::new(),
            series_entries: Vec::new(),
            current_file_name: None,
            position_secs: 0.0,
            duration_secs: None,
            is_playing: false,
            blocking_users: Vec::new(),
            bg_hash_progress: None,
        }
    }
}

/// Build display data from app state and storage.
///
/// Caller must hold `app_state` lock while calling this. The storage mutex
/// is acquired briefly inside.
pub fn build_display_data(
    app: &AppState,
    storage: &std::sync::Mutex<ClientStorage>,
    bg_hash_progress: &Arc<BgHashProgress>,
) -> DisplayData {
    let crdt = app.sync_engine.state();

    // Chat messages
    let chat_view = crdt.chat.merged_view();
    let chat_messages: Vec<(UserId, String)> = chat_view
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
        .and_then(|fid| {
            crdt.file_states
                .read(&(app.our_user_id.clone(), fid))
                .cloned()
        })
        .unwrap_or(FileState::Ready);
    user_entries.push(UserDisplayEntry {
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
        user_entries.push(UserDisplayEntry {
            name: user_id.0.clone(),
            user_state,
            file_state,
            is_self: false,
        });
    }

    // Playlist entries
    let playlist_snapshot = crdt.playlist.snapshot();
    let playlist_entries: Vec<PlaylistDisplayEntry> = playlist_snapshot
        .iter()
        .enumerate()
        .map(|(i, file_id)| {
            let local_path = storage
                .lock()
                .ok()
                .and_then(|s| s.get_file_mapping(file_id).ok().flatten());
            let crdt_filename = crdt.filenames.read(file_id).cloned();
            let display_name =
                file_display_name(file_id, local_path.as_deref(), crdt_filename.as_deref());
            let is_missing = local_path.is_none();
            PlaylistDisplayEntry {
                file_id: *file_id,
                display_name,
                is_missing,
                is_current: i == 0,
            }
        })
        .collect();

    // Series list
    let series_entries = storage
        .lock()
        .ok()
        .map(|st| series_browser::build_series_list(crdt, &st))
        .unwrap_or_default();

    let current_file_name = playlist_entries.first().map(|e| e.display_name.clone());

    let position_secs = app.our_position_secs;
    let duration_secs = app.file_duration_secs;
    let is_playing = app.playback.should_play;
    let blocking_users: Vec<String> = app
        .playback
        .blocking_users
        .iter()
        .map(|u| u.0.clone())
        .collect();

    let bg_hash = {
        let total_files = bg_hash_progress.total_files.load(Ordering::Relaxed);
        let completed_files = bg_hash_progress.completed_files.load(Ordering::Relaxed);
        if total_files > 0 && completed_files < total_files {
            let total_bytes = bg_hash_progress.total_bytes.load(Ordering::Relaxed);
            let completed_bytes = bg_hash_progress.completed_bytes.load(Ordering::Relaxed);
            let (rate_bps, eta_secs) = bg_hash_progress
                .rate_tracker
                .lock()
                .ok()
                .map(|tracker| {
                    let rate = tracker.current_rate_bps();
                    let eta = tracker.eta().map(|d| d.as_secs_f64());
                    (rate, eta)
                })
                .unwrap_or((None, None));
            Some(BgHashDisplayData {
                completed_files,
                total_files,
                completed_bytes,
                total_bytes,
                rate_bps,
                eta_secs,
            })
        } else {
            None
        }
    };

    DisplayData {
        chat_messages,
        user_entries,
        playlist_entries,
        series_entries,
        current_file_name,
        position_secs,
        duration_secs,
        is_playing,
        blocking_users,
        bg_hash_progress: bg_hash,
    }
}

/// Determine the display name for a file.
///
/// Priority: local path filename > CRDT shared filename > hex hash.
pub fn file_display_name(
    file_id: &FileId,
    local_path: Option<&Path>,
    crdt_filename: Option<&str>,
) -> String {
    if let Some(path) = local_path {
        return path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| format!("{file_id}"));
    }
    if let Some(name) = crdt_filename {
        return name.to_string();
    }
    format!("{file_id}")
}
