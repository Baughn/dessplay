//! The UI's Elm-style message and action types.
//!
//! Components produce [`Msg`]s from input events; the dispatcher's
//! `update()` turns them into internal state changes or [`UserAction`]s
//! that leave the UI toward the actor system (ui-architecture.md).

use std::path::PathBuf;

use dessplay_core::franchise::FranchiseKey;
use dessplay_core::types::{
    AniDbSeriesId, Ed2kHash, ListEntryId, NextEpState, SeriesListEntry, UserId,
};

use crate::config::Settings;

/// Messages produced by components.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    // Chat
    /// Send a chat message (already stripped of /commands).
    SendChat(String),
    /// A `/command` was entered.
    Command(String),

    // Series pane
    /// Cycle Recent -> All -> The List.
    CycleSeriesMode,
    /// Toggle title/year sort (All mode).
    ToggleSeriesSort,
    /// Toggle recency/alphabetical sort (The List mode).
    ToggleListSort,
    /// The Recent/All filter text changed (typed/backspace/clear).
    SeriesFilterChanged,
    /// Open a franchise (episode browser).
    BrowseFranchise(FranchiseKey),
    /// Edit a List entry.
    EditListEntry(ListEntryId),
    /// Enter on a List entry: for a linked entry whose franchise holds
    /// files, the episode browser (matched by component membership — the
    /// linked season is often not the franchise root, and may itself be
    /// file-less); otherwise the candidate-ranked disambiguation view
    /// for its next episode (design.md, Advancing next_ep); falls back
    /// to `EditListEntry` when nothing clears the bar. Never a silent
    /// no-op.
    BrowseListEntry(ListEntryId),
    /// Link a List entry to AniDB (opens the search modal).
    LinkListEntry(ListEntryId),

    // Users pane
    /// Mark a user Away (or clear an Away).
    ToggleAway(UserId),
    /// Mark a user NotWatching for the now-playing series (design.md
    /// #7/#13 — the "Kim tool"): attributed to us, since the target may be
    /// absent.
    SetNotWatching(UserId),

    // Playlist pane
    /// Set now-playing to this entry.
    PlaySelected(Ed2kHash),
    /// Open the file browser to add after this entry (`None` = append
    /// from the [Add New] row, which anchors to the end).
    AddFileAfter(Option<Ed2kHash>),
    /// Open the Nyaa search/active-import modal, anchored after this row.
    OpenNyaa(Option<Ed2kHash>),
    /// Episode browser: add this file (by hash) to the playlist. The file
    /// may or may not be held locally; if not, it downloads.
    EpisodeChosen {
        /// The chosen file's ed2k hash.
        hash: Ed2kHash,
    },
    /// Episode browser (`w`): cycle a file's group watched flag
    /// (design.md #10).
    ToggleEpisodeWatched {
        /// The file whose watched flag to flip.
        hash: Ed2kHash,
    },
    /// Move the selected entry after its successor (down).
    MoveDown(Ed2kHash),
    /// Move the selected entry before its predecessor (up).
    MoveUp(Ed2kHash),
    /// Tombstone an entry.
    RemoveEntry(Ed2kHash),
    /// Open the manual-mapping browser for a playlist entry (`M`).
    MapFile(Ed2kHash),
    /// Archive the selected cached file into the library (`A`).
    ArchiveFile(Ed2kHash),
    /// Cycle this entry's series watch state (`w`): Watching -> Maybe ->
    /// NotWatching -> ...
    CycleSeriesWatch(Ed2kHash),

    // Modals
    /// Close the topmost modal.
    CloseModal,
    /// File browser: a file was chosen (to hash + add to playlist).
    FileChosen {
        /// The chosen file.
        path: PathBuf,
        /// Insert after this entry (`None` = front... `None` from the
        /// [Add New] row means append; the dispatcher resolves it).
        after: Option<Ed2kHash>,
    },
    /// File browser: the sort toggle was pressed (design.md #8).
    ToggleBrowserSort,
    /// Directory picker: a directory was chosen (settings media root).
    DirChosen(PathBuf),
    /// Settings modal wants a directory picker on top.
    OpenDirPicker,
    /// Settings modal: save these settings + media roots.
    SettingsSaved(Box<Settings>, Vec<PathBuf>),
    /// List edit modal: save this entry, plus the edited progress register
    /// (`Some` only when next_ep/available actually changed — that register
    /// is written separately so it never clobbers a concurrent auto-advance).
    ListEntrySaved(ListEntryId, Box<SeriesListEntry>, Option<Box<NextEpState>>),
    /// AniDB search modal: run this search.
    AniDbSearchRequested(String),
    /// AniDB search modal: link the entry to this series.
    ListEntryLinked(ListEntryId, AniDbSeriesId),
    /// Mapping browser: the user picked a local file for an entry.
    FileMapped {
        /// The playlist entry being mapped.
        file: Ed2kHash,
        /// The chosen local file.
        path: PathBuf,
    },
    /// Nyaa modal: execute the current query.
    NyaaSearchRequested(String),
    /// Nyaa modal: download the selected inspected result.
    NyaaResultChosen {
        /// Inspected single-file search result.
        result: crate::torrent::nyaa::NyaaBrowseResult,
        /// Playlist anchor captured when search opened.
        after: Option<Ed2kHash>,
    },
    /// Nyaa active-import list: cancel the selected download.
    CancelNyaaImport(crate::torrent::engine::TorrentImportId),
    /// Nyaa active-import list: switch to a fresh search field.
    NewNyaaSearch,

    // Navigation / global
    /// Cycle pane focus.
    FocusNext,
    /// Cycle the subtitle mode (Off -> Intermixed -> Separate -> Off).
    CycleSubtitleMode,
    /// Quit DessPlay.
    Quit,
    /// Nothing to do (consumed internally by the component).
    None,
}

impl Msg {
    /// The variant's name, for logging. Deliberately omits payloads:
    /// `SettingsSaved` carries the password.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Msg::SendChat(_) => "SendChat",
            Msg::Command(_) => "Command",
            Msg::CycleSeriesMode => "CycleSeriesMode",
            Msg::ToggleSeriesSort => "ToggleSeriesSort",
            Msg::ToggleListSort => "ToggleListSort",
            Msg::SeriesFilterChanged => "SeriesFilterChanged",
            Msg::BrowseFranchise(_) => "BrowseFranchise",
            Msg::EditListEntry(_) => "EditListEntry",
            Msg::BrowseListEntry(_) => "BrowseListEntry",
            Msg::LinkListEntry(_) => "LinkListEntry",
            Msg::AniDbSearchRequested(_) => "AniDbSearchRequested",
            Msg::ListEntryLinked(..) => "ListEntryLinked",
            Msg::ToggleAway(_) => "ToggleAway",
            Msg::SetNotWatching(_) => "SetNotWatching",
            Msg::PlaySelected(_) => "PlaySelected",
            Msg::AddFileAfter(_) => "AddFileAfter",
            Msg::OpenNyaa(_) => "OpenNyaa",
            Msg::EpisodeChosen { .. } => "EpisodeChosen",
            Msg::ToggleEpisodeWatched { .. } => "ToggleEpisodeWatched",
            Msg::MoveDown(_) => "MoveDown",
            Msg::MoveUp(_) => "MoveUp",
            Msg::RemoveEntry(_) => "RemoveEntry",
            Msg::MapFile(_) => "MapFile",
            Msg::ArchiveFile(_) => "ArchiveFile",
            Msg::CycleSeriesWatch(_) => "CycleSeriesWatch",
            Msg::FileMapped { .. } => "FileMapped",
            Msg::NyaaSearchRequested(_) => "NyaaSearchRequested",
            Msg::NyaaResultChosen { .. } => "NyaaResultChosen",
            Msg::CancelNyaaImport(_) => "CancelNyaaImport",
            Msg::NewNyaaSearch => "NewNyaaSearch",
            Msg::CloseModal => "CloseModal",
            Msg::FileChosen { .. } => "FileChosen",
            Msg::ToggleBrowserSort => "ToggleBrowserSort",
            Msg::DirChosen(_) => "DirChosen",
            Msg::OpenDirPicker => "OpenDirPicker",
            Msg::SettingsSaved(..) => "SettingsSaved",
            Msg::ListEntrySaved(..) => "ListEntrySaved",
            Msg::FocusNext => "FocusNext",
            Msg::CycleSubtitleMode => "CycleSubtitleMode",
            Msg::Quit => "Quit",
            Msg::None => "None",
        }
    }
}

/// What a file browser is being opened for. The UI sends this with
/// [`UserAction::Browse`]; the main loop echoes it back alongside the
/// library listing so the answer opens the right browser.
#[derive(Clone, Debug, PartialEq)]
pub enum BrowseRequest {
    /// The playlist-add browser (`a`, or Enter on [Add New]).
    Add {
        /// Anchor entry; `None` = append at the end.
        after: Option<Ed2kHash>,
    },
    /// The manual-mapping browser (`M`).
    Map {
        /// The playlist entry being mapped.
        file: Ed2kHash,
        /// Target filename, for the edit-distance sort.
        target: String,
        /// Series key for the per-series last-used directory.
        series: Option<crate::storage::SeriesKey>,
    },
}

/// Actions leaving the UI toward the main loop / actors.
///
/// `Mutate`'s `Mutation` payload is the largest variant by a wide margin
/// (it carries whole `SeriesListEntry`/`NextEpState` values for List
/// writes) -- deliberately not boxed. These are one-at-a-time UI actions
/// dispatched at human-interaction speed, never stored in bulk, so boxing
/// would only relocate the allocation to every construction site (~60 of
/// them) for no real benefit.
#[derive(Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum UserAction {
    /// Apply a state mutation through the sync actor.
    Mutate(crate::actors::sync::Mutation),
    /// Open a file browser: the UI has no storage access, so the main
    /// loop gathers the library listing (and the watched set, and the
    /// mapping browser's start directory) and answers with
    /// [`crate::ui::shell::UiInput::Browse`].
    Browse(BrowseRequest),
    /// Hash this file (blocking work) and add it to the playlist after
    /// the anchor.
    HashAndAdd {
        /// File path.
        path: PathBuf,
        /// Anchor entry; `None` = append at the end.
        after: Option<Ed2kHash>,
    },
    /// Add a file to the playlist by hash, using the synced file catalog
    /// for its identity (the user may not hold it locally).
    AddByHash {
        /// The file's ed2k hash.
        hash: Ed2kHash,
        /// Anchor entry; `None` = append at the end.
        after: Option<Ed2kHash>,
    },
    /// Search Nyaa's anime category.
    SearchNyaa {
        /// Free-form query.
        query: String,
    },
    /// Start a selected single-file torrent import.
    StartNyaaImport {
        /// Local pending-import identity.
        id: crate::torrent::engine::TorrentImportId,
        /// Inspected search result.
        result: crate::torrent::nyaa::NyaaBrowseResult,
        /// Playlist anchor captured when search opened.
        after: Option<Ed2kHash>,
    },
    /// Cancel a pending Nyaa import.
    CancelNyaaImport {
        /// Local pending-import identity.
        id: crate::torrent::engine::TorrentImportId,
    },
    /// Persist settings + media roots.
    SaveSettings(Box<Settings>, Vec<PathBuf>),
    /// Ask the server for an AniDB name search (results come back as a
    /// UI input).
    AniDbSearch {
        /// The query.
        query: String,
    },
    /// Manually set a file's group watched flag (design.md #10). The
    /// server owns the watched flag (like `EofReached`), so this is sent
    /// straight to it rather than a CRDT `Mutation`.
    MarkWatched {
        /// The file.
        file: Ed2kHash,
        /// The new value.
        watched: bool,
    },
    /// Persist a manual mapping (playlist entry → local file the user
    /// picked) and resolve it.
    MapFile {
        /// The playlist entry.
        file: Ed2kHash,
        /// The chosen local file.
        path: PathBuf,
        /// Series key for remembering this directory; `None` when the
        /// entry has no metadata yet.
        series: Option<crate::storage::SeriesKey>,
    },
    /// Archive a cached file into the library under the download root.
    Archive {
        /// The cached file.
        file: Ed2kHash,
        /// Series name for the subdirectory (from metadata).
        series_name: Option<String>,
        /// Original filename.
        filename: String,
        /// Whether to place the file in a series-name subdirectory.
        subdirectory: bool,
    },
    /// Post a local-only system line to the chat log (command feedback,
    /// e.g. an unknown command or a `/skip` with no series info). The
    /// main loop stamps it with the shared clock — the UI has no clock of
    /// its own.
    Notice(String),
    /// Ping absent known users on IRC (design.md #4, `/summon`). The UI
    /// has no view of live IRC channel membership, so the main loop
    /// forwards this to the IRC actor, which resolves each username to a
    /// nick and reports the outcome back as a local system line.
    Summon(Vec<UserId>),
    /// Quit the application.
    Quit,
}
