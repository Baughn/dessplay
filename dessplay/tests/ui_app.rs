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
        view: std::sync::Arc::new(view),
        peers,
        recency: BTreeMap::new(),
        cache_hashes: Default::default(),
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
        recency: BTreeMap::new(),
        cache_hashes: cache.iter().copied().collect(),
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

/// Selecting a *different* entry pauses (EOF parity); re-selecting the
/// already-playing entry is not a transition, so it must not pause.
#[test]
fn selecting_now_playing_entry_does_not_pause() {
    let mut state = CrdtState::new();
    state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
    state.push_playlist_entry(A, ts(2), entry(2, "ep2.mkv"));
    state.set_now_playing(A, ts(3), Some(hash(1)));
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));

    for _ in 0..3 {
        ui.handle(key(Key::Tab)); // focus the Playlist pane
    }
    // Enter on the currently-playing row: re-assert now-playing only, no pause.
    assert_eq!(
        ui.handle(key(Key::Enter)),
        vec![UserAction::Mutate(Mutation::SetNowPlaying {
            file: Some(hash(1))
        })]
    );
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
    assert!(screen.contains("temporary"), "{screen}");
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
    // `M` opens the mapping browser (at the media roots).
    assert!(ui.handle(shift('M')).is_empty());
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

    ui.handle(key(Key::Tab)); // Series
    ui.handle(key(Key::Char('m'))); // All
    ui.handle(key(Key::Char('m'))); // The List
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

    ui.handle(key(Key::Tab)); // Series
    ui.handle(key(Key::Char('m'))); // All
    ui.handle(key(Key::Char('m'))); // The List
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
        },
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));

    ui.handle(key(Key::Tab)); // Series
    ui.handle(key(Key::Char('m'))); // All
    ui.handle(key(Key::Char('m'))); // The List
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
        },
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));
    ui.handle(key(Key::Tab));
    ui.handle(key(Key::Char('m')));
    ui.handle(key(Key::Char('m')));
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
