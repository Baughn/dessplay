//! Whole-app TUI tests: scripted key sequences through the real `Ui`
//! dispatcher, locator-style assertions on rendered TestBackend
//! buffers, insta snapshots for layout (testing-strategy.md).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use dessplay::actors::sync::Mutation;
use dessplay::config::Settings;
use dessplay::ui::app::{Ui, UiSnapshot};
use dessplay::ui::msg::UserAction;
use dessplay_core::net::{PeerInfo, Presence, Role};
use dessplay_core::playlist::NewPlaylistEntry;
use dessplay_core::types::{
    ActorId, Ed2kHash, ListEntryId, ListStatus, ManualState, SeriesListEntry, SharedTimestamp,
    UserId,
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
        view,
        peers,
        recency: BTreeMap::new(),
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

#[test]
fn first_run_opens_settings_modal() {
    let mut ui = Ui::new(
        UserId::new("kim"),
        Settings::default(), // no username/password: needs setup
        Vec::new(),
    );
    assert!(ui.modal_open());
    insta::assert_snapshot!(render(&mut ui, 100, 30));
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
    // Enter on the first row plays it.
    assert_eq!(
        ui.handle(key(Key::Enter)),
        vec![UserAction::Mutate(Mutation::SetNowPlaying {
            file: Some(hash(1))
        })]
    );
    // Move the second row up: it lands at the front (no anchor).
    ui.handle(key(Key::Down));
    assert_eq!(
        ui.handle(ctrl('k')),
        vec![UserAction::Mutate(Mutation::MovePlaylistAfter {
            hash: hash(2),
            anchor: None,
        })]
    );
    // Move it down instead: anchored after the third row.
    assert_eq!(
        ui.handle(ctrl('j')),
        vec![UserAction::Mutate(Mutation::MovePlaylistAfter {
            hash: hash(2),
            anchor: Some(hash(3)),
        })]
    );
    // Remove it.
    assert_eq!(
        ui.handle(key(Key::Char('d'))),
        vec![UserAction::Mutate(Mutation::RemovePlaylist {
            hash: hash(2)
        })]
    );
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
    // [Add New] is the only row in an empty playlist.
    assert!(ui.handle(key(Key::Enter)).is_empty());
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

    ui.handle(key(Key::Tab)); // Series
    ui.handle(key(Key::Char('m'))); // All
    ui.handle(key(Key::Char('m'))); // The List
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Watching (1)"), "{screen}");
    assert!(screen.contains("Frieren"), "{screen}");
    assert!(screen.contains("→12✓"), "{screen}");

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

#[test]
fn f2_toggles_subtitle_pane() {
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    assert!(!render(&mut ui, 100, 30).contains("Subtitles"));
    ui.handle(key(Key::Function(2)));
    assert!(render(&mut ui, 100, 30).contains("Subtitles"));
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
