//! The UI dispatcher: focus ring, modal stack, event routing, and the
//! Elm `update()` — the part of tui-realm's `Application` we replaced
//! with synchronous code so whole-app tests are deterministic
//! (ui-architecture.md, Framework Choice).

use std::collections::BTreeMap;
use std::path::PathBuf;

use dessplay_core::StateView;
use dessplay_core::franchise::{self, FranchiseKey};
use dessplay_core::net::PeerInfo;
use dessplay_core::types::{AniDbSeriesId, Ed2kHash, ManualState, UserId};
use tuirealm::component::AppComponent;
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers, NoUserEvent};
use tuirealm::props::{AttrValue, Attribute};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout};
use tuirealm::ratatui::widgets::{Block, Borders};

use super::components::{
    ChatPane, KeyBar, PlaylistPane, SeriesMode, SeriesPane, StatusBar, UsersPane,
};
use super::modals::{EpisodeBrowser, FileBrowser, ListEditModal, Season, SettingsModal};
use super::msg::{Msg, UserAction};
use super::props;
use crate::actors::sync::Mutation;
use crate::config::Settings;

/// Everything the UI renders from, refreshed on every state/peer
/// change.
#[derive(Clone, Debug, Default)]
pub struct UiSnapshot {
    /// The resolved CRDT view.
    pub view: StateView,
    /// The latest peer list.
    pub peers: Vec<PeerInfo>,
    /// Local watch history: series -> last-watched millis (drives the
    /// Recent mode sort).
    pub recency: BTreeMap<AniDbSeriesId, u64>,
}

/// Pane focus ring: Chat -> Series -> Users -> Playlist.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Focus {
    Chat,
    Series,
    Users,
    Playlist,
}

impl Focus {
    fn next(self) -> Self {
        match self {
            Focus::Chat => Focus::Series,
            Focus::Series => Focus::Users,
            Focus::Users => Focus::Playlist,
            Focus::Playlist => Focus::Chat,
        }
    }
}

/// An open modal.
enum Modal {
    Files(FileBrowser),
    Settings(SettingsModal),
    Episodes(EpisodeBrowser),
    ListEdit(ListEditModal),
}

impl Modal {
    fn as_component(&mut self) -> &mut dyn AppComponent<Msg, NoUserEvent> {
        match self {
            Modal::Files(modal) => modal,
            Modal::Settings(modal) => modal,
            Modal::Episodes(modal) => modal,
            Modal::ListEdit(modal) => modal,
        }
    }

    fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        match self {
            Modal::Files(modal) => modal.keybindings(),
            Modal::Settings(modal) => modal.keybindings(),
            Modal::Episodes(modal) => modal.keybindings(),
            Modal::ListEdit(modal) => modal.keybindings(),
        }
    }
}

/// The whole TUI.
pub struct Ui {
    me: UserId,
    chat: ChatPane,
    series: SeriesPane,
    users: UsersPane,
    playlist: PlaylistPane,
    status: StatusBar,
    keybar: KeyBar,
    modals: Vec<Modal>,
    focus: Focus,
    subtitle_pane: bool,
    snapshot: UiSnapshot,
    settings: Settings,
    media_roots: Vec<PathBuf>,
}

impl Ui {
    /// Build the UI. Opens the settings modal when the *given* settings
    /// need setup. Callers that prefill values (the `$USER` username,
    /// the `.env` password) but still want first-run confirmation use
    /// [`Ui::with_setup`].
    pub fn new(me: UserId, settings: Settings, media_roots: Vec<PathBuf>) -> Self {
        let open_settings = settings.needs_setup() || media_roots.is_empty();
        Self::with_setup(me, settings, media_roots, open_settings)
    }

    /// Build the UI, opening the settings modal iff `open_settings`
    /// (prefilled values appear as editable defaults).
    pub fn with_setup(
        me: UserId,
        settings: Settings,
        media_roots: Vec<PathBuf>,
        open_settings: bool,
    ) -> Self {
        let mut ui = Self {
            me,
            chat: ChatPane::default(),
            series: SeriesPane::default(),
            users: UsersPane::default(),
            playlist: PlaylistPane::default(),
            status: StatusBar::default(),
            keybar: KeyBar::default(),
            modals: Vec::new(),
            focus: Focus::Chat,
            subtitle_pane: settings.subtitle_pane,
            snapshot: UiSnapshot::default(),
            settings: settings.clone(),
            media_roots: media_roots.clone(),
        };
        if open_settings {
            ui.modals
                .push(Modal::Settings(SettingsModal::new(settings, media_roots)));
        }
        ui.sync_focus_attr();
        ui.refresh_keybar();
        ui
    }

    /// Replace the snapshot and recompute every pane's props.
    pub fn apply_snapshot(&mut self, snapshot: UiSnapshot) {
        self.chat.set_lines(props::chat_lines(&snapshot.view));
        self.users
            .set_props(props::users_props(&snapshot.view, &snapshot.peers));
        self.playlist
            .set_props(props::playlist_props(&snapshot.view, &self.me));
        self.status.set_props(props::status_props(
            &snapshot.view,
            &snapshot.peers,
            &self.me,
        ));
        self.snapshot = snapshot;
        self.refresh_series();
    }

    fn refresh_series(&mut self) {
        match self.series.mode() {
            SeriesMode::Recent => self.series.set_franchises(props::franchise_rows(
                &self.snapshot.view,
                self.series.sort(),
                Some(&self.snapshot.recency),
            )),
            SeriesMode::All => self.series.set_franchises(props::franchise_rows(
                &self.snapshot.view,
                self.series.sort(),
                None,
            )),
            SeriesMode::TheList => self
                .series
                .set_groups(props::list_groups(&self.snapshot.view)),
        }
    }

    fn sync_focus_attr(&mut self) {
        use tuirealm::component::Component;
        for (pane, focused) in [
            (
                &mut self.chat as &mut dyn Component,
                self.focus == Focus::Chat,
            ),
            (
                &mut self.series as &mut dyn Component,
                self.focus == Focus::Series,
            ),
            (
                &mut self.users as &mut dyn Component,
                self.focus == Focus::Users,
            ),
            (
                &mut self.playlist as &mut dyn Component,
                self.focus == Focus::Playlist,
            ),
        ] {
            pane.attr(
                Attribute::Focus,
                AttrValue::Flag(focused && self.modals.is_empty()),
            );
        }
    }

    fn refresh_keybar(&mut self) {
        let mut items: Vec<(&'static str, &'static str)> = match self.modals.last() {
            Some(modal) => modal.keybindings(),
            None => {
                let mut items = match self.focus {
                    Focus::Chat => self.chat.keybindings(),
                    Focus::Series => self.series.keybindings(),
                    Focus::Users => self.users.keybindings(),
                    Focus::Playlist => self.playlist.keybindings(),
                };
                items.insert(0, ("Tab", "Next pane"));
                items
            }
        };
        items.push(("C-c", "Quit"));
        self.keybar.set_items(items);
    }

    /// Route one input event; returns the actions it produced.
    pub fn handle(&mut self, ev: Event<NoUserEvent>) -> Vec<UserAction> {
        // Globals first.
        if let Event::Keyboard(KeyEvent {
            code: Key::Char('c'),
            modifiers,
        }) = &ev
            && *modifiers == KeyModifiers::CONTROL
        {
            return vec![UserAction::Quit];
        }
        if self.modals.is_empty() {
            match super::components::plain(&ev) {
                Some(Key::Tab) => {
                    self.focus = self.focus.next();
                    self.sync_focus_attr();
                    self.refresh_keybar();
                    return Vec::new();
                }
                Some(Key::Function(2)) => {
                    self.subtitle_pane = !self.subtitle_pane;
                    return Vec::new();
                }
                _ => {}
            }
        }

        let msg = match self.modals.last_mut() {
            Some(modal) => modal.as_component().on(&ev),
            None => match self.focus {
                Focus::Chat => self.chat.on(&ev),
                Focus::Series => self.series.on(&ev),
                Focus::Users => self.users.on(&ev),
                Focus::Playlist => self.playlist.on(&ev),
            },
        };
        let action = msg.and_then(|msg| self.update(msg));
        self.refresh_keybar();
        action.into_iter().collect()
    }

    /// The Elm update: messages become internal changes or actions.
    fn update(&mut self, msg: Msg) -> Option<UserAction> {
        match msg {
            Msg::None => None,
            Msg::SendChat(text) => Some(UserAction::Mutate(Mutation::Chat { text })),
            Msg::Command(command) => self.command(&command),
            Msg::CycleSeriesMode | Msg::ToggleSeriesSort => {
                self.refresh_series();
                None
            }
            Msg::BrowseFranchise(key) => {
                self.open_episode_browser(key);
                None
            }
            Msg::EditListEntry(id) => {
                let entry = self.snapshot.view.list_entries.get(&id)?.clone();
                self.modals
                    .push(Modal::ListEdit(ListEditModal::new(id, entry)));
                None
            }
            Msg::ToggleAway(user) => {
                let currently_away = matches!(
                    self.snapshot.view.manual_override.get(&user),
                    Some(Some(ManualState::Away { .. }))
                );
                let state = if currently_away {
                    None
                } else {
                    Some(ManualState::Away {
                        set_by: self.me.clone(),
                    })
                };
                Some(UserAction::Mutate(Mutation::SetManualOverride {
                    user,
                    state,
                }))
            }
            Msg::PlaySelected(hash) => Some(UserAction::Mutate(Mutation::SetNowPlaying {
                file: Some(hash),
            })),
            Msg::AddFileAfter(after) => {
                self.modals.push(Modal::Files(FileBrowser::for_file(
                    self.media_roots.clone(),
                    after,
                )));
                None
            }
            Msg::MoveUp(hash) => {
                let index = self.playlist_index(hash)?;
                if index == 0 {
                    return None;
                }
                let anchor = (index >= 2).then(|| self.snapshot.view.playlist[index - 2].hash);
                Some(UserAction::Mutate(Mutation::MovePlaylistAfter {
                    hash,
                    anchor,
                }))
            }
            Msg::MoveDown(hash) => {
                let index = self.playlist_index(hash)?;
                let anchor = self.snapshot.view.playlist.get(index + 1)?.hash;
                Some(UserAction::Mutate(Mutation::MovePlaylistAfter {
                    hash,
                    anchor: Some(anchor),
                }))
            }
            Msg::RemoveEntry(hash) => Some(UserAction::Mutate(Mutation::RemovePlaylist { hash })),
            Msg::CloseModal => {
                self.modals.pop();
                self.sync_focus_attr();
                None
            }
            Msg::FileChosen { path, after } => {
                self.modals.pop();
                self.sync_focus_attr();
                // `None` (from [Add New]) appends.
                let after =
                    after.or_else(|| self.snapshot.view.playlist.last().map(|entry| entry.hash));
                Some(UserAction::HashAndAdd { path, after })
            }
            Msg::OpenDirPicker => {
                self.modals.push(Modal::Files(FileBrowser::for_directory()));
                None
            }
            Msg::DirChosen(path) => {
                self.modals.pop();
                if let Some(Modal::Settings(settings)) = self.modals.last_mut() {
                    settings.add_root(path);
                }
                None
            }
            Msg::SettingsSaved(settings, roots) => {
                self.modals.pop();
                self.sync_focus_attr();
                self.settings = (*settings).clone();
                self.subtitle_pane = settings.subtitle_pane;
                self.media_roots = roots.clone();
                // First-run setup may have changed who we are.
                if let Some(name) = &settings.username {
                    self.me = UserId::new(name.clone());
                }
                Some(UserAction::SaveSettings(settings, roots))
            }
            Msg::ListEntrySaved(id, entry) => {
                self.modals.pop();
                self.sync_focus_attr();
                Some(UserAction::Mutate(Mutation::PutListEntry {
                    id,
                    entry: *entry,
                }))
            }
            Msg::FocusNext => {
                self.focus = self.focus.next();
                self.sync_focus_attr();
                None
            }
            Msg::ToggleSubtitlePane => {
                self.subtitle_pane = !self.subtitle_pane;
                None
            }
            Msg::Quit => Some(UserAction::Quit),
        }
    }

    fn playlist_index(&self, hash: Ed2kHash) -> Option<usize> {
        self.snapshot
            .view
            .playlist
            .iter()
            .position(|entry| entry.hash == hash)
    }

    /// `/commands` from the chat input.
    fn command(&mut self, command: &str) -> Option<UserAction> {
        let mut parts = command.split_whitespace();
        match parts.next()? {
            "/quit" | "/exit" | "/q" => Some(UserAction::Quit),
            "/afk" => {
                let name = parts.next()?;
                Some(UserAction::Mutate(Mutation::SetManualOverride {
                    user: UserId::new(name),
                    state: Some(ManualState::Away {
                        set_by: self.me.clone(),
                    }),
                }))
            }
            _ => None,
        }
    }

    fn open_episode_browser(&mut self, key: FranchiseKey) {
        let view = &self.snapshot.view;
        let franchise = franchise::franchises(view)
            .into_iter()
            .find(|franchise| franchise.key == key);
        let Some(franchise) = franchise else { return };
        let filename = |hash: &Ed2kHash| {
            view.playlist
                .iter()
                .find(|entry| entry.hash == *hash)
                .map(|entry| entry.state.filename.clone())
                .unwrap_or_else(|| hash.to_string())
        };
        let seasons: Vec<Season> = if franchise.series.is_empty() {
            vec![Season {
                title: franchise.title.clone(),
                episodes: franchise
                    .files
                    .iter()
                    .map(|hash| (*hash, filename(hash)))
                    .collect(),
            }]
        } else {
            franchise
                .series
                .iter()
                .map(|series| Season {
                    title: view
                        .series_relations
                        .get(series)
                        .map(|relations| relations.title.clone())
                        .unwrap_or_else(|| format!("anidb:{}", series.0)),
                    episodes: view
                        .anidb_metadata
                        .iter()
                        .filter_map(|(hash, metadata)| {
                            let metadata = metadata.as_ref()?;
                            (metadata.series_id == Some(*series)).then(|| (*hash, filename(hash)))
                        })
                        .collect(),
                })
                .collect()
        };
        self.modals.push(Modal::Episodes(EpisodeBrowser::new(
            franchise.title,
            seasons,
        )));
    }

    /// Render the whole screen (design.md, TUI Layout).
    pub fn draw(&mut self, frame: &mut Frame) {
        use tuirealm::component::Component;
        let [main, status_area, keybar_area] = Layout::vertical([
            Constraint::Min(8),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(main);
        let [series_area, users_area, playlist_area] = Layout::vertical([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .areas(right);

        if self.subtitle_pane {
            let [chat_area, subs_area] =
                Layout::vertical([Constraint::Percentage(70), Constraint::Percentage(30)])
                    .areas(left);
            self.chat.view(frame, chat_area);
            frame.render_widget(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(super::theme::dim())
                    .title("Subtitles"),
                subs_area,
            );
        } else {
            self.chat.view(frame, left);
        }
        self.series.view(frame, series_area);
        self.users.view(frame, users_area);
        self.playlist.view(frame, playlist_area);
        self.status.view(frame, status_area);
        self.keybar.view(frame, keybar_area);
        if let Some(modal) = self.modals.last_mut() {
            modal.as_component().view(frame, frame.area());
        }
    }

    /// Is a modal open? (Tests and the shell.)
    pub fn modal_open(&self) -> bool {
        !self.modals.is_empty()
    }

    /// Current settings (the shell persists them on save).
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
}
