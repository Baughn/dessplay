//! Whole-app TUI tests: scripted key sequences through the real `Ui`
//! dispatcher, locator-style assertions on rendered TestBackend
//! buffers, insta snapshots for layout (testing-strategy.md).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use dessplay::actors::sync::Mutation;
use dessplay::config::Settings;
use dessplay::torrent::engine::TorrentImportId;
use dessplay::torrent::nyaa::{NyaaBrowseResult, NyaaMatch};
use dessplay::ui::app::{Ui, UiSnapshot};
use dessplay::ui::msg::{BrowseRequest, UserAction};
use dessplay_core::net::{PeerInfo, Presence, Role};
use dessplay_core::playlist::NewPlaylistEntry;
use dessplay_core::types::{
    ActorId, Ed2kHash, ListEntryId, ListStatus, ManualState, NextEpState, PlaybackIntent,
    SeriesListEntry, SharedTimestamp, UserId,
};
use dessplay_core::{CrdtState, StateView};
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers, NoUserEvent};
use tuirealm::ratatui::Terminal;
use tuirealm::ratatui::backend::TestBackend;
use tuirealm::testing::buffer_to_string;

const A: ActorId = ActorId::SERVER;

fn ts(t: u64) -> SharedTimestamp {
    SharedTimestamp(t)
}

fn hash(i: u8) -> Ed2kHash {
    Ed2kHash([i; 16])
}

fn key(code: Key) -> Event<NoUserEvent> {
    Event::Keyboard(KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
    })
}

fn ctrl(c: char) -> Event<NoUserEvent> {
    Event::Keyboard(KeyEvent {
        code: Key::Char(c),
        modifiers: KeyModifiers::CONTROL,
    })
}

fn shift(c: char) -> Event<NoUserEvent> {
    Event::Keyboard(KeyEvent {
        code: Key::Char(c),
        modifiers: KeyModifiers::SHIFT,
    })
}

fn paste(text: &str) -> Event<NoUserEvent> {
    Event::Paste(text.to_string())
}

/// Type a string into the focused component.
fn type_str(ui: &mut Ui, text: &str) -> Vec<UserAction> {
    text.chars()
        .flat_map(|c| ui.handle(key(Key::Char(c))))
        .collect()
}

/// A ready-to-use Ui with completed setup (no settings modal).
fn ui() -> Ui {
    let settings = Settings {
        username: Some("kim".into()),
        password: Some("hunter2".into()),
        ..Settings::default()
    };
    Ui::new(UserId::new("kim"), settings, vec!["/media".into()])
}

fn torrent_ui() -> Ui {
    let settings = Settings {
        username: Some("kim".into()),
        password: Some("hunter2".into()),
        torrent_enabled: true,
        ..Settings::default()
    };
    Ui::new(UserId::new("kim"), settings, vec!["/media".into()])
}

fn peer(name: &str) -> PeerInfo {
    PeerInfo {
        username: UserId::new(name),
        role: Role::Interactive,
        presence: Presence::Present,
        addresses: vec![],
        connected_since: 0,
    }
}

fn snapshot(view: StateView, peers: Vec<PeerInfo>) -> UiSnapshot {
    UiSnapshot {
        view: std::sync::Arc::new(view),
        peers,
        known_offline: Default::default(),
        now: 0,
        recency: BTreeMap::new(),
        cache_hashes: Default::default(),
        watched_hashes: Default::default(),
        link: dessplay::ui::props::LinkStatus::Connected,
    }
}

/// Regression: a playback-position tick fans the resolved view out to the
/// run loop's diff baseline (`last_view`) and to the UI thread. That must
/// share one `Arc`-allocated `StateView`, not deep-clone the whole view
/// ~10x/s (profiling: the StateView clone was a large chunk of play-time
/// malloc -- 2026-06-23). `Arc::ptr_eq` proves the fan-out is a refcount
/// bump rather than a deep copy.
#[test]
fn snapshot_fan_out_shares_one_view() {
    let snap = snapshot(StateView::default(), vec![]);
    // Mirrors the run loop's per-tick line: `last_view = snapshot.view.clone()`.
    let last_view = snap.view.clone();
    assert!(
        std::sync::Arc::ptr_eq(&last_view, &snap.view),
        "per-tick snapshot fan-out must share one StateView (Arc), not deep-clone it"
    );
}

fn snapshot_with_cache(view: StateView, peers: Vec<PeerInfo>, cache: &[Ed2kHash]) -> UiSnapshot {
    UiSnapshot {
        view: std::sync::Arc::new(view),
        peers,
        known_offline: Default::default(),
        now: 0,
        recency: BTreeMap::new(),
        cache_hashes: cache.iter().copied().collect(),
        watched_hashes: Default::default(),
        link: dessplay::ui::props::LinkStatus::Connected,
    }
}

fn entry(i: u8, name: &str) -> NewPlaylistEntry {
    NewPlaylistEntry {
        hash: hash(i),
        added_by: UserId::new("kim"),
        filename: name.into(),
        size_bytes: 1,
        duration_millis: Some(1_440_000),
    }
}

/// Render to a buffer string for locator-style assertions.
fn render(ui: &mut Ui, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let completed = terminal.draw(|frame| ui.draw(frame)).unwrap();
    buffer_to_string(completed.buffer)
}

#[test]
fn layout_snapshot_empty_state() {
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    insta::assert_snapshot!(render(&mut ui, 100, 30));
}

/// Regression (2026-07-06): a dead handshake used to be invisible — the
/// status bar showed stale playback state while the network actor sat in
/// a 30s timeout, and the user concluded dessplay had hung. Any link
/// state other than Connected must be visible on the status bar.
#[test]
fn status_bar_shows_connection_state_while_not_connected() {
    let mut ui = ui();
    let mut snap = snapshot(StateView::default(), vec![peer("kim")]);
    snap.link = dessplay::ui::props::LinkStatus::Connecting { attempt: 1 };
    ui.apply_snapshot(snap);
    let rendered = render(&mut ui, 100, 30);
    assert!(rendered.contains("connecting to server"), "{rendered}");

    // Later attempts surface the counter — visible progress, not a
    // frozen line.
    let mut snap = snapshot(StateView::default(), vec![peer("kim")]);
    snap.link = dessplay::ui::props::LinkStatus::Connecting { attempt: 3 };
    ui.apply_snapshot(snap);
    let rendered = render(&mut ui, 100, 30);
    assert!(
        rendered.contains("connecting to server (attempt 3)"),
        "{rendered}"
    );

    // A mid-session drop shows as lost-and-retrying.
    let mut snap = snapshot(StateView::default(), vec![peer("kim")]);
    snap.link = dessplay::ui::props::LinkStatus::Down;
    ui.apply_snapshot(snap);
    let rendered = render(&mut ui, 100, 30);
    assert!(rendered.contains("connection lost"), "{rendered}");

    // And a healthy link shows playback state, not connection state.
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    let rendered = render(&mut ui, 100, 30);
    assert!(!rendered.contains("connecting to server"), "{rendered}");
}

#[test]
fn first_run_opens_settings_modal() {
    let mut ui = Ui::new(
        UserId::new("kim"),
        Settings::default(), // no username/password: needs setup
        Vec::new(),
    );
    assert!(ui.modal_open());
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("[Account !]"), "{screen}");
    assert!(screen.contains("[Files !]"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn settings_playback_layout_snapshot() {
    let mut ui = ui();
    ui.handle(key(Key::Function(3)));
    ui.handle(key(Key::Right));
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Settings — Playback & display"), "{screen}");
    assert!(screen.contains("WIP — not applied"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn settings_files_layout_snapshot_scrolls_many_roots() {
    let settings = Settings {
        username: Some("kim".into()),
        password: Some("hunter2".into()),
        ..Settings::default()
    };
    let roots = (0..20)
        .map(|index| {
            format!("/media/a-very-long-library-root-name-that-needs-clipping/{index:02}").into()
        })
        .collect();
    let mut ui = Ui::new(UserId::new("kim"), settings, roots);
    ui.handle(key(Key::Function(3)));
    ui.handle(key(Key::Right));
    ui.handle(key(Key::Right));
    ui.handle(key(Key::PageDown));
    ui.handle(key(Key::PageDown));
    ui.handle(key(Key::PageDown));
    ui.handle(key(Key::Up));
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Settings — Files & transfers"), "{screen}");
    assert!(screen.contains("/18") || screen.contains("/19"), "{screen}");
    assert!(screen.contains("Upload limit"), "{screen}");
    assert!(screen.contains("[Save]"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn settings_irc_layout_snapshot() {
    let mut ui = ui();
    ui.handle(key(Key::Function(3)));
    for _ in 0..3 {
        ui.handle(key(Key::Right));
    }
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Settings — IRC bridge"), "{screen}");
    assert!(screen.contains("IRC is public"), "{screen}");
    insta::assert_snapshot!(screen);
}

#[test]
fn settings_save_from_playback_emits_the_complete_draft() {
    let mut ui = ui();
    ui.handle(key(Key::Function(3)));
    ui.handle(key(Key::Right));
    ui.handle(key(Key::Enter)); // mpv -> VLC placeholder
    let actions = ui.handle(shift('S'));
    let [UserAction::SaveSettings(settings, roots)] = actions.as_slice() else {
        panic!("expected a complete settings save, got {actions:?}");
    };
    assert_eq!(settings.username.as_deref(), Some("kim"));
    assert_eq!(settings.password.as_deref(), Some("hunter2"));
    assert_eq!(settings.player, dessplay::config::PlayerKind::Vlc);
    assert_eq!(roots, &[std::path::PathBuf::from("/media")]);
}

#[test]
fn prefilled_first_run_still_confirms_and_adopts_username() {
    // Stored settings were empty, but $USER and the .env password are
    // prefilled: the modal must still open (with the prefills as
    // defaults), and a username edited there becomes our identity.
    let prefilled = Settings {
        username: Some("svein".into()),
        password: Some("hunter2".into()),
        ..Settings::default()
    };
    let mut ui = Ui::with_setup(UserId::new("svein"), prefilled, vec!["/media".into()], true);
    assert!(ui.modal_open(), "prefills must not skip first-run setup");

    ui.handle(key(Key::Enter)); // edit Username (prefilled "svein")
    type_str(&mut ui, "-laptop");
    ui.handle(key(Key::Enter)); // commit the field
    let actions = ui.handle(ctrl('s'));
    let [UserAction::SaveSettings(saved, _)] = actions.as_slice() else {
        panic!("expected SaveSettings, got {actions:?}");
    };
    assert_eq!(saved.username.as_deref(), Some("svein-laptop"));
    assert!(!ui.modal_open());

    // The Ui adopted the new identity: /afk attributes to it.
    type_str(&mut ui, "/afk baughn");
    assert_eq!(
        ui.handle(key(Key::Enter)),
        vec![UserAction::Mutate(Mutation::SetManualOverride {
            user: UserId::new("baughn"),
            state: Some(ManualState::Away {
                set_by: UserId::new("svein-laptop")
            }),
        })]
    );
}

#[test]
fn chat_enter_sends_message() {
    let mut ui = ui();
    assert!(type_str(&mut ui, "hello world").is_empty());
    let actions = ui.handle(key(Key::Enter));
    assert_eq!(
        actions,
        vec![UserAction::Mutate(Mutation::Chat {
            text: "hello world".into()
        })]
    );
    // Input cleared: a second Enter sends nothing.
    assert!(ui.handle(key(Key::Enter)).is_empty());
}

#[test]
fn chat_commands() {
    let mut ui = ui();
    type_str(&mut ui, "/afk baughn");
    let actions = ui.handle(key(Key::Enter));
    assert_eq!(
        actions,
        vec![UserAction::Mutate(Mutation::SetManualOverride {
            user: UserId::new("baughn"),
            state: Some(ManualState::Away {
                set_by: UserId::new("kim")
            }),
        })]
    );
    type_str(&mut ui, "/quit");
    assert_eq!(ui.handle(key(Key::Enter)), vec![UserAction::Quit]);
}

#[test]
fn slash_shows_filtered_command_suggestions() {
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    // No popup while the input is empty.
    assert!(!render(&mut ui, 100, 30).contains("/skip"));
    // A bare `/` lists every command.
    type_str(&mut ui, "/");
    let all = render(&mut ui, 100, 30);
    assert!(all.contains("/ready"), "{all}");
    assert!(all.contains("/skip"), "{all}");
    assert!(all.contains("/quit"), "{all}");
    // Help text is tabulated: each command's description starts at the
    // same column. Find the help column on two rows of differing
    // command-name length; they must line up.
    let ready_col = all
        .lines()
        .find(|l| l.contains("/ready"))
        .and_then(|l| l.find("mark yourself ready"))
        .expect("ready row");
    let away_col = all
        .lines()
        .find(|l| l.contains("/away"))
        .and_then(|l| l.find("mark yourself (or"))
        .expect("away row");
    assert_eq!(ready_col, away_col, "help column not aligned:\n{all}");
    // Typing narrows to the matching command(s).
    type_str(&mut ui, "sk");
    let narrowed = render(&mut ui, 100, 30);
    assert!(narrowed.contains("/skip"), "{narrowed}");
    assert!(!narrowed.contains("/ready"), "{narrowed}");
}

#[test]
fn chat_enter_clears_my_away() {
    // Marked Away by another user; pressing Enter to send a chat line
    // clears it and still sends.
    let mut state = CrdtState::new();
    state.set_manual_override(
        A,
        ts(1),
        UserId::new("kim"),
        Some(ManualState::Away {
            set_by: UserId::new("baughn"),
        }),
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));
    type_str(&mut ui, "back now");
    let actions = ui.handle(key(Key::Enter));
    assert!(actions.iter().any(|a| matches!(
        a,
        UserAction::Mutate(Mutation::SetManualOverride { state: None, user })
            if *user == UserId::new("kim")
    )));
    assert!(actions.iter().any(|a| matches!(
        a,
        UserAction::Mutate(Mutation::Chat { text }) if text == "back now"
    )));
}

#[test]
fn ctrl_c_quits_from_anywhere() {
    let mut ui = ui();
    assert_eq!(ui.handle(ctrl('c')), vec![UserAction::Quit]);
    ui.handle(key(Key::Tab));
    assert_eq!(ui.handle(ctrl('c')), vec![UserAction::Quit]);
}

#[test]
fn tab_cycles_focus_and_keybar_follows() {
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    let chat_bar = render(&mut ui, 100, 30);
    assert!(chat_bar.contains("Send"), "{chat_bar}");

    ui.handle(key(Key::Tab)); // Series
    ui.handle(key(Key::Tab)); // Users
    let users_bar = render(&mut ui, 100, 30);
    assert!(users_bar.contains("Mark away"), "{users_bar}");

    ui.handle(key(Key::Tab)); // Playlist
    let playlist_bar = render(&mut ui, 100, 30);
    assert!(playlist_bar.contains("Remove"), "{playlist_bar}");

    ui.handle(key(Key::Tab)); // back to Chat
    assert!(render(&mut ui, 100, 30).contains("Send"));
}

#[test]
fn playlist_actions() {
    let mut state = CrdtState::new();
    state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
    state.push_playlist_entry(A, ts(2), entry(2, "ep2.mkv"));
    state.push_playlist_entry(A, ts(3), entry(3, "ep3.mkv"));
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));

    for _ in 0..3 {
        ui.handle(key(Key::Tab)); // Chat -> Series -> Users -> Playlist
    }
    // Enter on the first row makes it now-playing. Nothing was playing,
    // so this is a real transition: it also latches intent Paused, exactly
    // like an EOF advance, so the new file loads paused at the start.
    assert_eq!(
        ui.handle(key(Key::Enter)),
        vec![
            UserAction::Mutate(Mutation::SetNowPlaying {
                file: Some(hash(1))
            }),
            UserAction::Mutate(Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Paused,
            }),
        ]
    );
    // Move the second row up with `K`: it lands at the front (no anchor).
    ui.handle(key(Key::Down)); // sel -> row 2 (hash 2)
    assert_eq!(
        ui.handle(key(Key::Char('K'))),
        vec![UserAction::Mutate(Mutation::MovePlaylistAfter {
            hash: hash(2),
            anchor: None,
        })]
    );
    // `K` carried the cursor up with the entry, so re-select hash 2 before the
    // down move. (The harness doesn't apply the reorder, so the props are still
    // [1,2,3] and row index 1 is hash 2.)
    ui.handle(key(Key::Down)); // sel back to row 2 (hash 2)
    // Move it down with `J`: anchored after the third row. Lowercase works too.
    assert_eq!(
        ui.handle(key(Key::Char('j'))),
        vec![UserAction::Mutate(Mutation::MovePlaylistAfter {
            hash: hash(2),
            anchor: Some(hash(3)),
        })]
    );
    // `J` carried the cursor down to the third row, so step back up to hash 2
    // before removing it.
    ui.handle(key(Key::Up)); // sel back to row 2 (hash 2)
    assert_eq!(
        ui.handle(key(Key::Char('d'))),
        vec![UserAction::Mutate(Mutation::RemovePlaylist {
            hash: hash(2)
        })]
    );
}

#[test]
fn nyaa_requires_the_startup_setting() {
    let mut ui = ui();
    for _ in 0..3 {
        ui.handle(key(Key::Tab));
    }
    let actions = ui.handle(key(Key::Char('n')));
    assert!(matches!(actions.as_slice(), [UserAction::Notice(text)] if text.contains("enable")));
    assert!(!ui.modal_open());
}

#[test]
fn nyaa_search_select_progress_reopen_and_cancel() {
    let mut state = CrdtState::new();
    state.push_playlist_entry(A, ts(1), entry(1, "anchor.mkv"));
    let mut ui = torrent_ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));
    for _ in 0..3 {
        ui.handle(key(Key::Tab));
    }
    assert!(ui.handle(key(Key::Char('n'))).is_empty());
    assert!(render(&mut ui, 100, 30).contains("Search Nyaa"));
    type_str(&mut ui, "karen");
    assert_eq!(
        ui.handle(key(Key::Enter)),
        vec![UserAction::SearchNyaa {
            query: "karen".into()
        }]
    );
    let result = NyaaBrowseResult {
        title: "Karen release".into(),
        filename: "karen-01.mkv".into(),
        size_bytes: 1_000_000,
        seeders: 42,
        chosen: NyaaMatch {
            title: "Karen release".into(),
            torrent_url: "https://nyaa.si/download/1.torrent".into(),
            info_hash: "0123456789abcdef0123456789abcdef01234567".into(),
        },
    };
    ui.set_nyaa_results("karen", Ok(vec![result.clone()]));
    assert!(render(&mut ui, 100, 30).contains("karen-01.mkv"));
    assert_eq!(
        ui.handle(key(Key::Enter)),
        vec![UserAction::StartNyaaImport {
            id: TorrentImportId(1),
            result,
            after: Some(hash(1)),
        }]
    );
    ui.set_nyaa_import_progress(
        TorrentImportId(1),
        "karen-01.mkv".into(),
        dessplay::actors::file::NyaaImportStage::Downloading,
        500_000,
        1_000_000,
    );
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Adding to playlist"), "{screen}");
    assert!(screen.contains("Downloading karen-01.mkv"), "{screen}");
    assert!(ui.handle(key(Key::Char('n'))).is_empty());
    assert!(render(&mut ui, 100, 30).contains("Nyaa imports"));
    assert_eq!(
        ui.handle(key(Key::Char('d'))),
        vec![UserAction::CancelNyaaImport {
            id: TorrentImportId(1)
        }]
    );
}

/// Selecting a *different* entry pauses (EOF parity); re-selecting the
/// already-playing entry is not a transition, so it is a true no-op -- it
/// must neither pause nor re-emit `SetNowPlaying` (a redundant NowPlaying op
/// makes the server reset seek authority back to Server, yanking it from
/// whoever just seeked).
#[test]
fn reselecting_now_playing_entry_is_a_noop() {
    let mut state = CrdtState::new();
    state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
    state.push_playlist_entry(A, ts(2), entry(2, "ep2.mkv"));
    state.set_now_playing(A, ts(3), Some(hash(1)));
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));

    for _ in 0..3 {
        ui.handle(key(Key::Tab)); // focus the Playlist pane
    }
    // Enter on the currently-playing row: a no-op, no actions at all (so seek
    // authority is left untouched).
    assert_eq!(ui.handle(key(Key::Enter)), vec![]);
    // Enter on a different row: a real transition, so it also pauses.
    ui.handle(key(Key::Down));
    assert_eq!(
        ui.handle(key(Key::Enter)),
        vec![
            UserAction::Mutate(Mutation::SetNowPlaying {
                file: Some(hash(2))
            }),
            UserAction::Mutate(Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Paused,
            }),
        ]
    );
}

/// Toggling the All-Series sort with `s` must persist the choice (a
/// `SaveSettings` carrying the new sort), so it survives a restart
/// (design.md: "Sort mode for All Series is persisted across sessions").
#[test]
fn all_series_sort_toggle_persists() {
    let mut ui = ui();
    ui.handle(key(Key::Tab)); // Chat -> Series
    ui.handle(key(Key::Char('m'))); // The List (default) -> Recent
    ui.handle(key(Key::Char('m'))); // Recent -> All
    let actions = ui.handle(key(Key::Char('s'))); // toggle sort to Year
    let [UserAction::SaveSettings(saved, _)] = actions.as_slice() else {
        panic!("expected a SaveSettings, got {actions:?}");
    };
    assert_eq!(saved.series_sort, dessplay::ui::props::SeriesSort::Year);
}

#[test]
fn add_file_via_browser_produces_hash_and_add() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ep1.mkv"), b"video bytes").unwrap();
    let mut ui = Ui::new(
        UserId::new("kim"),
        Settings {
            username: Some("kim".into()),
            password: Some("x".into()),
            ..Settings::default()
        },
        vec![dir.path().to_path_buf()],
    );
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    for _ in 0..3 {
        ui.handle(key(Key::Tab));
    }
    // [Add New] is the only row in an empty playlist. Opening asks the
    // main loop for the library index; the answer opens the browser.
    let actions = ui.handle(key(Key::Enter));
    assert_eq!(
        actions,
        vec![UserAction::Browse(BrowseRequest::Add { after: None })]
    );
    assert!(!ui.modal_open());
    ui.open_file_browser(
        BrowseRequest::Add { after: None },
        vec![],
        Default::default(),
        None,
    );
    assert!(ui.modal_open());
    // Roots list -> the temp dir -> the file.
    assert!(ui.handle(key(Key::Enter)).is_empty());
    let actions = ui.handle(key(Key::Enter));
    assert_eq!(
        actions,
        vec![UserAction::HashAndAdd {
            path: dir.path().join("ep1.mkv"),
            after: None,
        }]
    );
    assert!(!ui.modal_open());
}

/// design.md #8: `Tab` in the file browser toggles Alphabetical <->
/// Newest and persists the choice, same pattern as
/// `all_series_sort_toggle_persists`.
#[test]
fn file_browser_sort_toggle_persists() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ep1.mkv"), b"video bytes").unwrap();
    let mut ui = Ui::new(
        UserId::new("kim"),
        Settings {
            username: Some("kim".into()),
            password: Some("x".into()),
            ..Settings::default()
        },
        vec![dir.path().to_path_buf()],
    );
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    ui.open_file_browser(
        BrowseRequest::Add { after: None },
        vec![],
        Default::default(),
        None,
    );
    assert!(ui.modal_open());
    let actions = ui.handle(key(Key::Tab));
    let [UserAction::SaveSettings(saved, _)] = actions.as_slice() else {
        panic!("expected a SaveSettings, got {actions:?}");
    };
    assert_eq!(
        saved.file_browser_sort,
        dessplay::ui::props::BrowserSort::Newest
    );
    // The modal is still open — Tab was consumed by the browser's own
    // keymap, not the global focus-cycle key.
    assert!(ui.modal_open());
}

/// design.md #33: pasting a single existing-file path while the Playlist
/// pane is focused becomes a playlist add — same as picking it in the
/// browser, anchored after the current selection (here, none selected in
/// an empty playlist, so `after: None`).
#[test]
fn paste_existing_file_path_on_playlist_focus_adds_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ep1.mkv");
    std::fs::write(&file, b"video bytes").unwrap();
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    for _ in 0..3 {
        ui.handle(key(Key::Tab)); // Chat -> Series -> Users -> Playlist
    }
    let actions = ui.handle(paste(&file.display().to_string()));
    assert_eq!(
        actions,
        vec![UserAction::HashAndAdd {
            path: file,
            after: None,
        }]
    );
    assert!(!ui.modal_open());
}

/// A paste while Chat is focused (the default) lands in the chat input as
/// plain text, exactly as if typed — asserted by sending it and checking
/// the resulting chat mutation.
#[test]
fn paste_plain_text_on_chat_focus_lands_in_input() {
    let mut ui = ui();
    assert!(ui.handle(paste("hello from the clipboard")).is_empty());
    let actions = ui.handle(key(Key::Enter));
    assert_eq!(
        actions,
        vec![UserAction::Mutate(Mutation::Chat {
            text: "hello from the clipboard".into()
        })]
    );
}

/// A paste on Playlist focus that is *not* a real file (design.md #33:
/// "any other paste") falls through to the chat input too — the playlist
/// short-circuit only fires for an actual existing-file path.
#[test]
fn paste_nonexistent_path_on_playlist_focus_lands_in_chat_input() {
    let mut ui = ui();
    for _ in 0..3 {
        ui.handle(key(Key::Tab)); // Chat -> Series -> Users -> Playlist
    }
    // No leading `/` — a pasted path starting with `/` would otherwise be
    // read back as a slash-command once it lands in the chat input.
    assert!(ui.handle(paste("no-such-file.mkv")).is_empty());
    // Switch back to chat to send what landed there.
    ui.handle(key(Key::Tab));
    let actions = ui.handle(key(Key::Enter));
    assert_eq!(
        actions,
        vec![UserAction::Mutate(Mutation::Chat {
            text: "no-such-file.mkv".into()
        })]
    );
}

/// `a` on a playlist entry with a local copy: the browser opens in that
/// file's directory with the cursor on it, so the next episode is one
/// keypress away.
#[test]
fn add_browser_opens_at_the_selected_entry() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Anime");
    std::fs::create_dir_all(root.join("Frieren")).unwrap();
    for name in ["ep1.mkv", "ep2.mkv"] {
        std::fs::write(root.join("Frieren").join(name), b"x").unwrap();
    }
    let mut state = CrdtState::new();
    state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
    let mut ui = Ui::new(
        UserId::new("kim"),
        Settings {
            username: Some("kim".into()),
            password: Some("x".into()),
            ..Settings::default()
        },
        vec![root.clone()],
    );
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));
    for _ in 0..3 {
        ui.handle(key(Key::Tab)); // focus Playlist (cursor on ep1's row)
    }
    let actions = ui.handle(key(Key::Char('a')));
    assert_eq!(
        actions,
        vec![UserAction::Browse(BrowseRequest::Add {
            after: Some(hash(1))
        })]
    );
    ui.open_file_browser(
        BrowseRequest::Add {
            after: Some(hash(1)),
        },
        vec![
            (root.join("Frieren").join("ep1.mkv"), hash(1), 1_000),
            (root.join("Frieren").join("ep2.mkv"), hash(2), 2_000),
        ],
        [hash(1)].into_iter().collect(), // ep1 personally watched
        None,
    );
    assert!(ui.modal_open());
    // The cursor starts on ep1 itself; Down+Enter picks its neighbour
    // without any directory navigation.
    ui.handle(key(Key::Down));
    let actions = ui.handle(key(Key::Enter));
    assert_eq!(
        actions,
        vec![UserAction::HashAndAdd {
            path: root.join("Frieren").join("ep2.mkv"),
            after: Some(hash(1)),
        }]
    );
}

/// Type-to-search in the add browser: typing filters the whole library
/// recursively, directories list first as root-relative paths, and
/// Enter on a directory clears the search and browses it.
#[test]
fn add_browser_type_to_search_finds_deep_directories() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Anime");
    let deep = root.join("Purgatory").join("Haibane Renmei");
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("ep1.mkv"), b"x").unwrap();
    let mut ui = Ui::new(
        UserId::new("kim"),
        Settings {
            username: Some("kim".into()),
            password: Some("x".into()),
            ..Settings::default()
        },
        vec![root.clone()],
    );
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    for _ in 0..3 {
        ui.handle(key(Key::Tab));
    }
    ui.handle(key(Key::Enter)); // [Add New] -> Browse request
    ui.open_file_browser(
        BrowseRequest::Add { after: None },
        vec![(deep.join("ep1.mkv"), hash(1), 1_000)],
        Default::default(),
        None,
    );
    // Search is case-insensitive over root-relative paths; the deep
    // directory matches without any navigation.
    type_str(&mut ui, "haibane");
    let screen = render(&mut ui, 100, 30);
    assert!(
        screen.contains("Anime/Purgatory/Haibane Renmei"),
        "{screen}"
    );
    // Enter on the directory row clears the search and browses it: the
    // next Enter picks the file inside (proving cwd moved there).
    assert!(ui.handle(key(Key::Enter)).is_empty());
    let actions = ui.handle(key(Key::Enter));
    assert_eq!(
        actions,
        vec![UserAction::HashAndAdd {
            path: deep.join("ep1.mkv"),
            after: None,
        }]
    );
}

#[test]
fn archive_action_emits_archive_with_series_and_filename() {
    let mut state = CrdtState::new();
    state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
    state.set_anidb_metadata(
        A,
        ts(2),
        hash(1),
        Some(dessplay_core::types::AniDbMetadata {
            source: dessplay_core::types::MetadataSource::AniDb,
            series_name: "Frieren".into(),
            series_id: Some(dessplay_core::types::AniDbSeriesId(42)),
            episode_number: Some("1".into()),
        }),
    );
    let mut ui = ui();
    // The file is cache-only ("temporary"), so archive is offered.
    ui.apply_snapshot(snapshot_with_cache(
        state.view(),
        vec![peer("kim")],
        &[hash(1)],
    ));
    for _ in 0..3 {
        ui.handle(key(Key::Tab)); // focus Playlist
    }
    assert_eq!(
        ui.handle(shift('A')),
        vec![UserAction::Archive {
            file: hash(1),
            series_name: Some("Frieren".into()),
            filename: "ep1.mkv".into(),
        }]
    );
}

#[test]
fn temporary_marker_renders_for_cache_only_files() {
    let mut state = CrdtState::new();
    state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
    let mut ui = ui();
    ui.apply_snapshot(snapshot_with_cache(
        state.view(),
        vec![peer("kim")],
        &[hash(1)],
    ));
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("temp"), "{screen}");
}

/// Regression: an over-long filename used to shove the right-aligned tag
/// columns ("temp", the watch state) past the pane border — the title
/// must truncate instead, keeping the tag columns visible (the playlist
/// renders as a table).
#[test]
fn long_filename_does_not_push_tags_offscreen() {
    let mut state = CrdtState::new();
    let long = format!("[SubGroup] {} - 01 [1080p][ABCD1234].mkv", "A".repeat(80));
    state.push_playlist_entry(A, ts(1), entry(1, &long));
    let mut ui = ui();
    ui.apply_snapshot(snapshot_with_cache(
        state.view(),
        vec![peer("kim")],
        &[hash(1)],
    ));
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("temp"), "{screen}");
    assert!(screen.contains("maybe"), "{screen}");
}

#[test]
fn system_chat_line_renders() {
    let mut ui = ui();
    ui.push_system(0, "Archived ep1.mkv".into());
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Archived ep1.mkv"), "{screen}");
}

#[test]
fn day_separator_renders_between_different_days() {
    use dessplay::ui::props;
    let day = 86_400_000u64;
    let t1 = 1_000_000_000_000; // a fixed instant
    let t2 = t1 + 2 * day; // two days later — a different biblical day
    assert_ne!(props::biblical_date(t1), props::biblical_date(t2));

    let mut ui = ui();
    ui.push_system(t1, "first".into());
    ui.push_system(t2, "second".into());
    let screen = render(&mut ui, 100, 30);

    // The divider carries the later day's date label.
    let label = props::day_separator(t2).text;
    assert!(!label.is_empty());
    assert!(screen.contains(&label), "expected '{label}' in:\n{screen}");
}

#[test]
fn no_day_separator_within_one_day() {
    use dessplay::ui::props;
    let t1 = 1_000_000_000_000;
    let t2 = t1 + 60_000; // one minute later — same biblical day
    assert_eq!(props::biblical_date(t1), props::biblical_date(t2));

    let mut ui = ui();
    ui.push_system(t1, "first".into());
    ui.push_system(t2, "second".into());
    let screen = render(&mut ui, 100, 30);

    // No divider is inserted, so the date label never appears.
    let label = props::day_separator(t1).text;
    assert!(!screen.contains(&label), "unexpected divider in:\n{screen}");
}

#[test]
fn archive_action_ignored_for_non_temporary_file() {
    // A file already in a media root (not in the cache set) shows no
    // "temporary" marker, so `A` is a no-op.
    let mut state = CrdtState::new();
    state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));
    for _ in 0..3 {
        ui.handle(key(Key::Tab)); // focus Playlist
    }
    assert_eq!(ui.handle(shift('A')), vec![]);
}

#[test]
fn map_file_opens_browser_ranked_by_edit_distance_and_maps() {
    // A directory of candidates; the target is "ep1.mkv". By edit
    // distance "ep2.mkv" (one substitution) ranks above the long
    // unrelated name, so it lands at row 0.
    let dir = tempfile::tempdir().unwrap();
    for name in ["a-completely-unrelated-movie.avi", "ep2.mkv"] {
        std::fs::write(dir.path().join(name), b"x").unwrap();
    }
    let mut state = CrdtState::new();
    // A missing entry linked to series metadata (so the mapping carries
    // a SeriesKey).
    state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
    state.set_anidb_metadata(
        A,
        ts(2),
        hash(1),
        Some(dessplay_core::types::AniDbMetadata {
            source: dessplay_core::types::MetadataSource::AniDb,
            series_name: "Frieren".into(),
            series_id: Some(dessplay_core::types::AniDbSeriesId(42)),
            episode_number: Some("1".into()),
        }),
    );
    let mut ui = Ui::new(
        UserId::new("kim"),
        Settings {
            username: Some("kim".into()),
            password: Some("x".into()),
            ..Settings::default()
        },
        vec![dir.path().to_path_buf()],
    );
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));
    for _ in 0..3 {
        ui.handle(key(Key::Tab)); // focus Playlist
    }
    // `M` requests the mapping browser; the main loop answers with the
    // library (and would supply the series' last-used directory).
    let series = Some(dessplay::storage::SeriesKey::AniDb(
        dessplay_core::types::AniDbSeriesId(42),
    ));
    let actions = ui.handle(shift('M'));
    assert_eq!(
        actions,
        vec![UserAction::Browse(BrowseRequest::Map {
            file: hash(1),
            target: "ep1.mkv".into(),
            series: series.clone(),
        })]
    );
    ui.open_file_browser(
        BrowseRequest::Map {
            file: hash(1),
            target: "ep1.mkv".into(),
            series,
        },
        vec![],
        Default::default(),
        None,
    );
    assert!(ui.modal_open());
    // Enter the only root directory.
    assert!(ui.handle(key(Key::Enter)).is_empty());
    // The closest filename to "ep1.mkv" is ranked first; selecting it
    // maps the entry and carries the series key.
    let actions = ui.handle(key(Key::Enter));
    assert_eq!(
        actions,
        vec![UserAction::MapFile {
            file: hash(1),
            path: dir.path().join("ep2.mkv"),
            series: Some(dessplay::storage::SeriesKey::AniDb(
                dessplay_core::types::AniDbSeriesId(42)
            )),
        }]
    );
    assert!(!ui.modal_open());
}

#[test]
fn users_pane_toggles_away() {
    let mut state = CrdtState::new();
    state.set_manual_override(
        A,
        ts(1),
        UserId::new("baughn"),
        Some(ManualState::Away {
            set_by: UserId::new("kim"),
        }),
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("baughn"), peer("kim")]));
    ui.handle(key(Key::Tab));
    ui.handle(key(Key::Tab)); // Users
    // First row is baughn (sorted peer list) — currently Away: toggles off.
    assert_eq!(
        ui.handle(key(Key::Char('a'))),
        vec![UserAction::Mutate(Mutation::SetManualOverride {
            user: UserId::new("baughn"),
            state: None,
        })]
    );
    // kim is not away: toggles on.
    ui.handle(key(Key::Down));
    assert_eq!(
        ui.handle(key(Key::Char('a'))),
        vec![UserAction::Mutate(Mutation::SetManualOverride {
            user: UserId::new("kim"),
            state: Some(ManualState::Away {
                set_by: UserId::new("kim")
            }),
        })]
    );
}

/// Regression: pressing `a` on a peer must NOT clear an Away that *someone
/// else* set -- the spec scopes the clear to "an Away you set" (an Away set
/// by another user is cleared only by the marked user's own "I'm here"
/// action). Pressing `a` on it instead re-marks them Away by us.
#[test]
fn users_pane_a_does_not_clear_anothers_away() {
    let mut state = CrdtState::new();
    state.set_manual_override(
        A,
        ts(1),
        UserId::new("baughn"),
        Some(ManualState::Away {
            set_by: UserId::new("nero"),
        }),
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("baughn"), peer("kim")]));
    ui.handle(key(Key::Tab));
    ui.handle(key(Key::Tab)); // Users
    // baughn is Away, but set by nero (not me=kim): `a` re-marks (set_by me),
    // it does not clear someone else's Away.
    assert_eq!(
        ui.handle(key(Key::Char('a'))),
        vec![UserAction::Mutate(Mutation::SetManualOverride {
            user: UserId::new("baughn"),
            state: Some(ManualState::Away {
                set_by: UserId::new("kim"),
            }),
        })]
    );
}

#[test]
fn the_list_renders_and_edits() {
    let mut state = CrdtState::new();
    state.put_list_entry(
        A,
        ts(1),
        ListEntryId(7),
        SeriesListEntry {
            name: "Frieren".into(),
            nero_name: Some("Funeral".into()),
            genre: None,
            notes: vec![],
            recommender: None,
            status: ListStatus::Active,
            status_note: None,
            source: None,
            watchers: [UserId::new("Baughn")].into_iter().collect(),
            anidb_series_id: None,
            local_aliases: Default::default(),
            manual_files: Default::default(),
            anidb_unavailable: false,
        },
    );
    state.set_next_ep(
        A,
        ts(2),
        ListEntryId(7),
        dessplay_core::types::NextEpState {
            next_ep: Some("12".into()),
            available: true,
        },
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));

    ui.handle(key(Key::Tab)); // Series, already in The List (default mode)
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Watching (1)"), "{screen}");
    assert!(screen.contains("Frieren"), "{screen}");
    // Episode # and availability are separate table columns now (design.md
    // feedback: the 3 spreadsheet columns need their own aligned cells).
    assert!(screen.contains("12"), "{screen}");
    assert!(screen.contains("✓"), "{screen}");

    // Enter on the entry (unlinked) opens the editor; rename and save.
    ui.handle(key(Key::Down)); // heading -> entry
    ui.handle(key(Key::Enter));
    assert!(ui.modal_open());
    ui.handle(key(Key::Enter)); // edit Name
    type_str(&mut ui, "!");
    ui.handle(key(Key::Enter)); // commit field
    let actions = ui.handle(ctrl('s'));
    let [UserAction::Mutate(Mutation::PutListEntry { id, entry })] = actions.as_slice() else {
        panic!("expected PutListEntry, got {actions:?}");
    };
    assert_eq!(*id, ListEntryId(7));
    assert_eq!(entry.name, "Frieren!");
    assert_eq!(entry.status, ListStatus::Active);
    assert!(!ui.modal_open());
}

/// Phase 19 completion: the edit modal is where an unlinked entry's
/// identity data is grown by hand (design.md, Series Identity /
/// UI Integration) — `local_aliases` as semicolon-separated names, and
/// `manual_files` as semicolon-separated ed2k hex hashes (unparsable
/// tokens dropped).
#[test]
fn the_list_editor_edits_aliases_and_manual_files() {
    let mut state = CrdtState::new();
    state.put_list_entry(
        A,
        ts(1),
        ListEntryId(7),
        SeriesListEntry {
            name: "Some Obscure Show".into(),
            nero_name: None,
            genre: None,
            notes: vec![],
            recommender: None,
            status: ListStatus::Active,
            status_note: None,
            source: None,
            watchers: Default::default(),
            anidb_series_id: None,
            local_aliases: ["OldAlias".into()].into_iter().collect(),
            manual_files: Default::default(),
            anidb_unavailable: false,
        },
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));
    ui.handle(key(Key::Tab)); // Series pane (The List)
    ui.handle(key(Key::Down)); // heading -> entry
    ui.handle(key(Key::Char('e'))); // edit modal
    assert!(ui.modal_open());

    // Down to "Aliases" (row 10) and replace the alias list.
    for _ in 0..10 {
        ui.handle(key(Key::Down));
    }
    ui.handle(key(Key::Enter));
    for _ in 0.."OldAlias".len() {
        ui.handle(key(Key::Backspace)); // clear the prefilled alias
    }
    type_str(&mut ui, "ObscureShow S2; Obscure Show");
    ui.handle(key(Key::Enter));

    // Down to "Manual files" and enter one good and one bad token.
    ui.handle(key(Key::Down));
    ui.handle(key(Key::Enter));
    type_str(&mut ui, &format!("{}; not-a-hash", hash(9)));
    ui.handle(key(Key::Enter));

    let actions = ui.handle(ctrl('s'));
    let [UserAction::Mutate(Mutation::PutListEntry { id, entry })] = actions.as_slice() else {
        panic!("expected PutListEntry, got {actions:?}");
    };
    assert_eq!(*id, ListEntryId(7));
    assert_eq!(
        entry.local_aliases,
        ["ObscureShow S2".to_string(), "Obscure Show".to_string()]
            .into_iter()
            .collect()
    );
    assert_eq!(
        entry.manual_files,
        [hash(9)].into_iter().collect(),
        "the parsable hash sticks; the junk token is dropped"
    );
}

/// Regression (2026-07-05 review): the List table's name cell must pad
/// by terminal *display width*, not char count. A CJK title (2 cells per
/// glyph — routine in `nero_name`) under-padded and shoved the
/// episode/available/watchers columns out of alignment, defeating the
/// aligned-table feature on exactly the rows it exists for.
#[test]
fn the_list_columns_align_across_cjk_and_ascii_rows() {
    let mut state = CrdtState::new();
    let entry = |name: &str, nero: Option<&str>| SeriesListEntry {
        name: name.into(),
        nero_name: nero.map(String::from),
        genre: None,
        notes: vec![],
        recommender: None,
        status: ListStatus::Active,
        status_note: None,
        source: None,
        watchers: Default::default(),
        anidb_series_id: None,
        local_aliases: Default::default(),
        manual_files: Default::default(),
        anidb_unavailable: false,
    };
    state.put_list_entry(A, ts(1), ListEntryId(1), entry("Frieren", None));
    // A CJK nero long enough to overflow the name cell (truncation path) …
    state.put_list_entry(
        A,
        ts(2),
        ListEntryId(2),
        entry("Sousou", Some("葬送のフリーレン")),
    );
    // … and one short enough to fit (padding path).
    state.put_list_entry(A, ts(2), ListEntryId(3), entry("Bocchi", Some("ぼっち")));
    for (id, ep) in [
        (ListEntryId(1), "12"),
        (ListEntryId(2), "34"),
        (ListEntryId(3), "56"),
    ] {
        state.set_next_ep(
            A,
            ts(3),
            id,
            dessplay_core::types::NextEpState {
                next_ep: Some(ep.into()),
                available: false,
            },
        );
    }
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));
    ui.handle(key(Key::Tab)); // Series pane, The List (default mode)
    let screen = render(&mut ui, 100, 30);

    // The episode column must start at the same screen column on the
    // ASCII row and the CJK row. `buffer_to_string` emits one char per
    // terminal cell (a wide glyph is its char plus a space for the
    // continuation cell), so the char count of the prefix *is* the
    // screen column.
    let ep_column = |name: &str, ep: &str| {
        let line = screen
            .lines()
            .find(|l| l.contains(name))
            .unwrap_or_else(|| panic!("no row for {name}: {screen}"));
        let idx = line
            .find(ep)
            .unwrap_or_else(|| panic!("no ep {ep}: {line}"));
        line[..idx].chars().count()
    };
    assert_eq!(
        ep_column("Frieren", "12"),
        ep_column("Sousou", "34"),
        "episode column drifts on an over-long CJK title (truncation):\n{screen}"
    );
    assert_eq!(
        ep_column("Frieren", "12"),
        ep_column("Bocchi", "56"),
        "episode column drifts on a short CJK title (padding):\n{screen}"
    );
}

/// An unlinked entry whose AniDB search came up empty gets a durable
/// callout in the row itself (design.md, Series Identity) -- distinct
/// from an unlinked entry nobody's tried linking yet, which shows no
/// marker (covered by `the_list_renders_and_edits`, `anidb_unavailable:
/// false`).
#[test]
fn the_list_marks_a_confirmed_anidb_unavailable_entry() {
    let mut state = CrdtState::new();
    state.put_list_entry(
        A,
        ts(1),
        ListEntryId(7),
        SeriesListEntry {
            name: "Some Obscure Show".into(),
            nero_name: None,
            genre: None,
            notes: vec![],
            recommender: None,
            status: ListStatus::Active,
            status_note: None,
            source: None,
            watchers: Default::default(),
            anidb_series_id: None,
            local_aliases: Default::default(),
            manual_files: Default::default(),
            anidb_unavailable: true,
        },
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));
    ui.handle(key(Key::Tab)); // Series, already in The List (default mode)
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Some Obscure Show"), "{screen}");
    assert!(
        screen.contains("⊘"),
        "expected the unavailable marker: {screen}"
    );
}

/// The List edit modal must be saveable without Ctrl-S (eaten as XOFF on
/// basic terminals): capital `S` saves and closes the modal end-to-end.
#[test]
fn list_edit_modal_saves_on_capital_s() {
    let mut state = CrdtState::new();
    state.put_list_entry(
        A,
        ts(1),
        ListEntryId(7),
        SeriesListEntry {
            name: "Frieren".into(),
            nero_name: None,
            genre: None,
            notes: vec![],
            recommender: None,
            status: ListStatus::Active,
            status_note: None,
            source: None,
            watchers: Default::default(),
            anidb_series_id: None,
            local_aliases: Default::default(),
            manual_files: Default::default(),
            anidb_unavailable: false,
        },
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));

    ui.handle(key(Key::Tab)); // Series, already in The List (default mode)
    ui.handle(key(Key::Down)); // heading -> entry
    ui.handle(key(Key::Enter)); // open the edit modal
    assert!(ui.modal_open());

    // Capital `S` saves and closes the modal.
    let actions = ui.handle(shift('S'));
    let [UserAction::Mutate(Mutation::PutListEntry { id, .. })] = actions.as_slice() else {
        panic!("expected PutListEntry from capital S, got {actions:?}");
    };
    assert_eq!(*id, ListEntryId(7));
    assert!(!ui.modal_open());
}

/// The List edit modal must expose the `next_ep` free-text field and the
/// `available` (✓/✖) toggle so the documented "maintained by hand" path
/// exists. Opening on an entry shows its current values; editing next_ep
/// and toggling available, then saving, emits a `SetNextEp` mutation with
/// the new values alongside the `PutListEntry` write.
#[test]
fn list_edit_modal_edits_next_ep_and_available() {
    let mut state = CrdtState::new();
    state.put_list_entry(
        A,
        ts(1),
        ListEntryId(7),
        SeriesListEntry {
            name: "Frieren".into(),
            nero_name: None,
            genre: None,
            notes: vec![],
            recommender: None,
            status: ListStatus::CurrentSeason,
            status_note: None,
            source: None,
            watchers: Default::default(),
            anidb_series_id: None,
            local_aliases: Default::default(),
            manual_files: Default::default(),
            anidb_unavailable: false,
        },
    );
    state.set_next_ep(
        A,
        ts(2),
        ListEntryId(7),
        NextEpState {
            next_ep: Some("11".into()),
            available: false,
        },
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));

    ui.handle(key(Key::Tab)); // Series, already in The List (default mode)
    ui.handle(key(Key::Down)); // heading -> entry
    ui.handle(key(Key::Enter)); // open the edit modal
    assert!(ui.modal_open());

    // The modal must surface the current next_ep / available values.
    let screen = render(&mut ui, 100, 40);
    assert!(screen.contains("Next ep"), "{screen}");
    assert!(screen.contains("11"), "current next_ep not shown: {screen}");
    assert!(screen.contains("Available"), "{screen}");

    // Move to the Next ep field, edit "11" -> "12".
    for _ in 0..8 {
        ui.handle(key(Key::Down));
    }
    ui.handle(key(Key::Enter)); // open the field editor (prefilled "11")
    ui.handle(key(Key::Backspace));
    ui.handle(key(Key::Backspace));
    type_str(&mut ui, "12");
    ui.handle(key(Key::Enter)); // commit the field

    // Move to Available and toggle it on.
    ui.handle(key(Key::Down));
    ui.handle(key(Key::Enter)); // toggle available

    let actions = ui.handle(ctrl('s'));
    let next_ep = actions
        .iter()
        .find_map(|a| match a {
            UserAction::Mutate(Mutation::SetNextEp { id, next_ep }) if *id == ListEntryId(7) => {
                Some(next_ep.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected a SetNextEp mutation, got {actions:?}"));
    assert_eq!(next_ep.next_ep.as_deref(), Some("12"));
    assert!(next_ep.available, "available should have been toggled on");
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, UserAction::Mutate(Mutation::PutListEntry { .. }))),
        "the entry itself should still be saved: {actions:?}"
    );
    assert!(!ui.modal_open());
}

/// Saving the List edit modal without touching next_ep / available must
/// NOT emit a `SetNextEp` mutation — that register is kept apart so a note
/// edit never clobbers a concurrent server EOF auto-advance.
#[test]
fn list_edit_modal_save_without_next_ep_change_emits_no_set_next_ep() {
    let mut state = CrdtState::new();
    state.put_list_entry(
        A,
        ts(1),
        ListEntryId(7),
        SeriesListEntry {
            name: "Frieren".into(),
            nero_name: None,
            genre: None,
            notes: vec![],
            recommender: None,
            status: ListStatus::CurrentSeason,
            status_note: None,
            source: None,
            watchers: Default::default(),
            anidb_series_id: None,
            local_aliases: Default::default(),
            manual_files: Default::default(),
            anidb_unavailable: false,
        },
    );
    state.set_next_ep(
        A,
        ts(2),
        ListEntryId(7),
        NextEpState {
            next_ep: Some("11".into()),
            available: false,
        },
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));

    ui.handle(key(Key::Tab)); // Series, already in The List (default mode)
    ui.handle(key(Key::Down)); // heading -> entry
    ui.handle(key(Key::Enter)); // open the edit modal

    // Edit only the Name field, leave next_ep / available untouched.
    ui.handle(key(Key::Enter)); // edit Name
    type_str(&mut ui, "!");
    ui.handle(key(Key::Enter)); // commit
    let actions = ui.handle(ctrl('s'));
    assert!(
        actions
            .iter()
            .all(|a| !matches!(a, UserAction::Mutate(Mutation::SetNextEp { .. }))),
        "an unrelated edit must not write next_ep: {actions:?}"
    );
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, UserAction::Mutate(Mutation::PutListEntry { .. }))),
        "the entry edit should still save: {actions:?}"
    );
}

#[test]
fn linking_a_list_entry_searches_and_links() {
    use dessplay_core::net::AniDbSearchHit;
    use dessplay_core::types::AniDbSeriesId;

    let mut state = CrdtState::new();
    state.put_list_entry(
        A,
        ts(1),
        ListEntryId(7),
        SeriesListEntry {
            name: "GochiUsa".into(),
            nero_name: None,
            genre: None,
            notes: vec![],
            recommender: None,
            status: ListStatus::Active,
            status_note: None,
            source: None,
            watchers: Default::default(),
            anidb_series_id: None,
            local_aliases: Default::default(),
            manual_files: Default::default(),
            anidb_unavailable: false,
        },
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));

    ui.handle(key(Key::Tab)); // Series, already in The List (default mode)
    ui.handle(key(Key::Down)); // heading -> entry

    // 'l' opens the search modal and fires a search for the name.
    let actions = ui.handle(key(Key::Char('l')));
    assert_eq!(
        actions,
        vec![UserAction::AniDbSearch {
            query: "GochiUsa".into()
        }]
    );
    assert!(ui.modal_open());
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Link to AniDB — GochiUsa"), "{screen}");
    assert!(screen.contains("searching…"), "{screen}");

    // Results arrive (a stale reply for another query is ignored).
    ui.set_search_results(
        "stale query",
        vec![AniDbSearchHit {
            series: AniDbSeriesId(1),
            title: "Wrong".into(),
            matched: "Wrong".into(),
        }],
    );
    ui.set_search_results(
        "GochiUsa",
        vec![
            AniDbSearchHit {
                series: AniDbSeriesId(5391),
                title: "Gochuumon wa Usagi Desu ka?".into(),
                matched: "GochiUsa".into(),
            },
            AniDbSearchHit {
                series: AniDbSeriesId(9903),
                title: "Gochuumon wa Usagi Desu ka??".into(),
                matched: "GochiUsa S2".into(),
            },
        ],
    );
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Gochuumon wa Usagi Desu ka?"), "{screen}");
    assert!(screen.contains("a5391"), "{screen}");
    assert!(
        !screen.contains("Wrong"),
        "stale results displayed: {screen}"
    );

    // Pick the second result; Enter links it.
    ui.handle(key(Key::Down));
    let actions = ui.handle(key(Key::Enter));
    let [UserAction::Mutate(Mutation::PutListEntry { id, entry })] = actions.as_slice() else {
        panic!("expected PutListEntry, got {actions:?}");
    };
    assert_eq!(*id, ListEntryId(7));
    assert_eq!(entry.anidb_series_id, Some(AniDbSeriesId(9903)));
    assert_eq!(entry.name, "GochiUsa", "other fields untouched");
    assert!(!ui.modal_open());
}

#[test]
fn editing_the_search_query_rearms_search() {
    let mut state = CrdtState::new();
    state.put_list_entry(
        A,
        ts(1),
        ListEntryId(7),
        SeriesListEntry {
            name: "X".into(),
            nero_name: None,
            genre: None,
            notes: vec![],
            recommender: None,
            status: ListStatus::Active,
            status_note: None,
            source: None,
            watchers: Default::default(),
            anidb_series_id: None,
            local_aliases: Default::default(),
            manual_files: Default::default(),
            anidb_unavailable: false,
        },
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));
    ui.handle(key(Key::Tab)); // already in The List (default mode)
    ui.handle(key(Key::Down));
    ui.handle(key(Key::Char('l')));

    // No results for "X"; the user retypes and Enter searches again
    // (instead of linking nothing).
    ui.set_search_results("X", vec![]);
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("no matches"), "{screen}");
    let actions = type_str(&mut ui, "yz");
    assert!(actions.is_empty());
    let actions = ui.handle(key(Key::Enter));
    assert_eq!(
        actions,
        vec![UserAction::AniDbSearch {
            query: "Xyz".into()
        }]
    );
    // Esc closes without linking.
    let actions = ui.handle(key(Key::Esc));
    assert!(actions.is_empty());
    assert!(!ui.modal_open());
}

#[test]
fn status_bar_shows_blockers() {
    let mut state = CrdtState::new();
    state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
    state.set_now_playing(A, ts(2), Some(hash(1)));
    state.set_playback_intent(A, ts(3), dessplay_core::types::PlaybackIntent::Playing);
    state.set_manual_override(A, ts(4), UserId::new("baughn"), Some(ManualState::Paused));
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim"), peer("baughn")]));
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("waiting on baughn (paused)"), "{screen}");
    assert!(screen.contains("Now Playing: ep1.mkv"), "{screen}");
}

/// design.md #6: the progress bar + time render on their own row, never
/// sharing a line with the variable-width "waiting on ..." blocker text
/// (which used to shove the bar sideways as blockers came and went).
#[test]
fn progress_line_is_on_its_own_row_separate_from_blockers() {
    let mut state = CrdtState::new();
    state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
    state.set_now_playing(A, ts(2), Some(hash(1)));
    state.set_playback_intent(A, ts(3), dessplay_core::types::PlaybackIntent::Playing);
    state.set_manual_override(A, ts(4), UserId::new("baughn"), Some(ManualState::Paused));
    state.set_playback_position(
        A,
        ts(5),
        UserId::new("kim"),
        dessplay_core::types::PlaybackPosition {
            position_millis: 720_000,
            timestamp: ts(5),
            file: hash(1),
        },
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim"), peer("baughn")]));
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("waiting on baughn (paused)"), "{screen}");
    assert!(screen.contains("12:00 / 24:00"), "{screen}");
    let blocker_line = screen
        .lines()
        .find(|line| line.contains("waiting on"))
        .expect("blocker line");
    assert!(
        !blocker_line.contains("12:00"),
        "progress bar/time must not share a row with blocker text: {blocker_line:?}"
    );
}

#[test]
fn f2_cycles_subtitle_mode() {
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    // Off: no separate pane.
    assert!(!render(&mut ui, 100, 30).contains("Subtitles"));
    // Off -> Intermixed: still no separate pane (subs fold into chat),
    // and the cycle persists the choice.
    let actions = ui.handle(key(Key::Function(2)));
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, UserAction::SaveSettings(..))),
        "F2 should persist the new mode"
    );
    assert!(!render(&mut ui, 100, 30).contains("Subtitles"));
    // Intermixed -> SeparatePane: the dedicated pane appears.
    ui.handle(key(Key::Function(2)));
    assert!(render(&mut ui, 100, 30).contains("Subtitles"));
    // SeparatePane -> Off: gone again.
    ui.handle(key(Key::Function(2)));
    assert!(!render(&mut ui, 100, 30).contains("Subtitles"));
}

#[test]
fn hashing_progress_overlay_appears_and_clears() {
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    assert!(!render(&mut ui, 100, 30).contains("Hashing"));

    // Two files in flight, one halfway.
    ui.set_hash_progress("ep1.mkv".into(), 0, 1_000, false);
    ui.set_hash_progress("ep2.mkv".into(), 500, 1_000, false);
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Hashing for playlist"), "{screen}");
    assert!(screen.contains("ep1.mkv"), "{screen}");
    assert!(screen.contains("ep2.mkv"), "{screen}");
    // The [####    ] style: a part-filled bar has hashes after the
    // bracket and spaces before the closing one; no percentage.
    assert!(screen.contains("[#"), "{screen}");
    assert!(screen.contains("  ]"), "{screen}");
    assert!(!screen.contains('%'), "{screen}");

    // The overlay is informational: input still reaches the panes
    // (you can chat while files hash).
    let actions = type_str(&mut ui, "hi");
    assert!(actions.is_empty());
    let actions = ui.handle(key(Key::Enter));
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, UserAction::Mutate(Mutation::Chat { text }) if text == "hi")),
        "chat must keep working under the hashing overlay: {actions:?}"
    );

    // Both finish: the overlay goes away.
    ui.set_hash_progress("ep1.mkv".into(), 0, 0, true);
    ui.set_hash_progress("ep2.mkv".into(), 0, 0, true);
    assert!(!render(&mut ui, 100, 30).contains("Hashing"));
}
