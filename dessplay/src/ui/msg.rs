//! The UI's Elm-style message and action types.
//!
//! Components produce [`Msg`]s from input events; the dispatcher's
//! `update()` turns them into internal state changes or [`UserAction`]s
//! that leave the UI toward the actor system (ui-architecture.md).

use std::path::PathBuf;

use dessplay_core::franchise::FranchiseKey;
use dessplay_core::types::{Ed2kHash, ListEntryId, SeriesListEntry, UserId};

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
    /// Open a franchise (episode browser).
    BrowseFranchise(FranchiseKey),
    /// Edit a List entry.
    EditListEntry(ListEntryId),

    // Users pane
    /// Mark a user Away (or clear an Away).
    ToggleAway(UserId),

    // Playlist pane
    /// Set now-playing to this entry.
    PlaySelected(Ed2kHash),
    /// Open the file browser to add after this entry (`None` = append
    /// from the [Add New] row, which anchors to the end).
    AddFileAfter(Option<Ed2kHash>),
    /// Move the selected entry after its successor (down).
    MoveDown(Ed2kHash),
    /// Move the selected entry before its predecessor (up).
    MoveUp(Ed2kHash),
    /// Tombstone an entry.
    RemoveEntry(Ed2kHash),

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
    /// Directory picker: a directory was chosen (settings media root).
    DirChosen(PathBuf),
    /// Settings modal wants a directory picker on top.
    OpenDirPicker,
    /// Settings modal: save these settings + media roots.
    SettingsSaved(Box<Settings>, Vec<PathBuf>),
    /// List edit modal: save this entry.
    ListEntrySaved(ListEntryId, Box<SeriesListEntry>),

    // Navigation / global
    /// Cycle pane focus.
    FocusNext,
    /// Toggle the subtitle pane.
    ToggleSubtitlePane,
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
            Msg::BrowseFranchise(_) => "BrowseFranchise",
            Msg::EditListEntry(_) => "EditListEntry",
            Msg::ToggleAway(_) => "ToggleAway",
            Msg::PlaySelected(_) => "PlaySelected",
            Msg::AddFileAfter(_) => "AddFileAfter",
            Msg::MoveDown(_) => "MoveDown",
            Msg::MoveUp(_) => "MoveUp",
            Msg::RemoveEntry(_) => "RemoveEntry",
            Msg::CloseModal => "CloseModal",
            Msg::FileChosen { .. } => "FileChosen",
            Msg::DirChosen(_) => "DirChosen",
            Msg::OpenDirPicker => "OpenDirPicker",
            Msg::SettingsSaved(..) => "SettingsSaved",
            Msg::ListEntrySaved(..) => "ListEntrySaved",
            Msg::FocusNext => "FocusNext",
            Msg::ToggleSubtitlePane => "ToggleSubtitlePane",
            Msg::Quit => "Quit",
            Msg::None => "None",
        }
    }
}

/// Actions leaving the UI toward the main loop / actors.
#[derive(Debug, PartialEq)]
pub enum UserAction {
    /// Apply a state mutation through the sync actor.
    Mutate(crate::actors::sync::Mutation),
    /// Hash this file (blocking work) and add it to the playlist after
    /// the anchor.
    HashAndAdd {
        /// File path.
        path: PathBuf,
        /// Anchor entry; `None` = append at the end.
        after: Option<Ed2kHash>,
    },
    /// Persist settings + media roots.
    SaveSettings(Box<Settings>, Vec<PathBuf>),
    /// Quit the application.
    Quit,
}
