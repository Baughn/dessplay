//! Whole-app TUI tests: scripted key sequences through the real `Ui`
//! dispatcher, locator-style assertions on rendered TestBackend
//! buffers, insta snapshots for layout (testing-strategy.md).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::num::NonZeroU64;

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
use tuirealm::event::{
    Event, Key, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind, NoUserEvent,
};
use tuirealm::ratatui::Terminal;
use tuirealm::ratatui::backend::TestBackend;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
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

fn click(column: u16, row: u16) -> Event<NoUserEvent> {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        modifiers: KeyModifiers::NONE,
        column,
        row,
    })
}

fn wheel(column: u16, row: u16, up: bool) -> Event<NoUserEvent> {
    Event::Mouse(MouseEvent {
        kind: if up {
            MouseEventKind::ScrollUp
        } else {
            MouseEventKind::ScrollDown
        },
        modifiers: KeyModifiers::NONE,
        column,
        row,
    })
}

/// The pane rectangles `Ui::draw` produces for a frame of this size —
/// the same Layout splits, so click coordinates in tests are derived,
/// not hand-counted. Returns (chat column, series, users, playlist).
fn pane_rects(width: u16, height: u16) -> (Rect, Rect, Rect, Rect) {
    let [main, _status, _keybar] = Layout::vertical([
        Constraint::Min(8),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(Rect::new(0, 0, width, height));
    // The main area's last row is the terminal-wide bottom line
    // (progress · suggestion · health); the panes split what's above.
    let [panes, _bottom] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(main);
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(panes);
    let [series, users, playlist] = Layout::vertical([
        Constraint::Percentage(34),
        Constraint::Percentage(33),
        Constraint::Percentage(33),
    ])
    .areas(right);
    (left, series, users, playlist)
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
        shared_now: 0,
        recency: BTreeMap::new(),
        cache_hashes: Default::default(),
        personal_watched: Default::default(),
        link: dessplay::ui::props::LinkStatus::Connected,
        health: dessplay::ui::props::HealthProps {
            link: dessplay::ui::props::LinkStatus::Connected,
            ..Default::default()
        },
    }
}

fn snapshot_with_cache(view: StateView, peers: Vec<PeerInfo>, cache: &[Ed2kHash]) -> UiSnapshot {
    UiSnapshot {
        view: std::sync::Arc::new(view),
        peers,
        known_offline: Default::default(),
        now: 0,
        shared_now: 0,
        recency: BTreeMap::new(),
        cache_hashes: cache.iter().copied().collect(),
        personal_watched: Default::default(),
        link: dessplay::ui::props::LinkStatus::Connected,
        health: Default::default(),
    }
}

fn entry(i: u8, name: &str) -> NewPlaylistEntry {
    NewPlaylistEntry {
        hash: hash(i),
        added_by: UserId::new("kim"),
        filename: name.into(),
        size_bytes: 1,
        duration_millis: NonZeroU64::new(1_440_000),
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

/// The playlist's bottom border sits level with the chat input's, and
/// the terminal-wide row below carries the bottom line — progress bar
/// left, health metrics right-aligned (design.md, Connection Health
/// Line). The one-row border asymmetry was the visible bug that
/// motivated the row.
#[test]
fn health_row_aligns_playlist_border_with_chat_input() {
    let (left, _series, _users, playlist) = pane_rects(100, 30);
    // The chat pane fills the whole left column, so its input's bottom
    // border is the column's last row; the playlist's bottom border
    // must land on that same row.
    let chat_input_bottom = left.y + left.height - 1;
    let playlist_bottom = playlist.y + playlist.height - 1;
    assert_eq!(playlist_bottom, chat_input_bottom);

    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    let rendered = render(&mut ui, 100, 30);
    let lines: Vec<&str> = rendered.lines().collect();
    let border_row: Vec<char> = lines[playlist_bottom as usize].chars().collect();
    assert_eq!(border_row[0], '└', "chat input bottom border");
    assert_eq!(border_row[50], '└', "playlist bottom border, same row");
    // The row below holds the bottom line, health right-aligned at the
    // terminal edge (nothing is measured yet in this snapshot).
    let bottom = lines[playlist_bottom as usize + 1];
    assert!(bottom.contains("link: measuring…"), "{rendered}");
    assert_eq!(
        bottom.chars().nth(99),
        Some('…'),
        "health hugs the terminal's right edge: {bottom:?}"
    );
}

/// The bottom line composes: health metrics right-aligned at the
/// terminal edge, the suggestion centered in the middle space with at
/// least two spaces of margin toward each neighbour.
#[test]
fn health_row_shows_metrics_and_right_aligned_suggestion() {
    use dessplay::ui::props::{
        HealthLevel, HealthProps, HealthSample, LinkStatus, SuggestionProps, Tone,
    };
    let mut ui = ui();
    let mut snap = snapshot(StateView::default(), vec![peer("kim")]);
    snap.health = HealthProps {
        link: LinkStatus::Connected,
        level: HealthLevel::Degraded,
        sample: Some(HealthSample {
            rtt_millis: Some(2_000),
            unanswered_probes: 0,
            server_silence_millis: 6_000,
            up_bps: 1_200_000,
            down_bps: 340_000,
        }),
        // Group playback: the 5s "worth showing" bar applies, so the
        // 6s silence renders as an age rather than "sync ok".
        playing: true,
        company: true,
        suggestion: Some(SuggestionProps {
            text: "high latency — disable BitTorrent (F3)".into(),
            tone: Tone::Paused,
        }),
    };
    ui.apply_snapshot(snap);
    let rendered = render(&mut ui, 200, 30);
    assert!(rendered.contains("▲1.2M ▼340K"), "{rendered}");
    assert!(rendered.contains("rtt 2000ms"), "{rendered}");
    assert!(rendered.contains("sync 6s"), "{rendered}");
    let row = rendered
        .lines()
        .find(|line| line.contains("disable BitTorrent"))
        .expect("suggestion rendered");
    // Health hugs the terminal's right edge ("sync 6s" is its last
    // fragment); the suggestion sits in the middle with ≥2 spaces of
    // margin on both sides.
    let chars: Vec<char> = row.chars().collect();
    assert_eq!(chars[199], 's', "health hugs the right edge: {row:?}");
    assert!(row.ends_with("sync 6s"), "{row:?}");
    assert!(
        row.contains("  high latency — disable BitTorrent (F3)  "),
        "suggestion has margin on both sides: {row:?}"
    );
    let start = row.find("high latency").expect("suggestion start");
    assert!(
        start > 40 && start < 120,
        "suggestion is roughly centered, not glued to an edge: start={start}"
    );

    // A 100-column terminal still fits everything here (no progress bar
    // in this snapshot); health stays right-aligned, suggestion whole.
    let narrow = render(&mut ui, 100, 30);
    assert!(narrow.contains("disable BitTorrent (F3)"), "{narrow}");
    assert!(narrow.contains("▲1.2M"), "{narrow}");
}

/// Regression (2026-08-12 review): the documented truncation order is
/// health > progress > suggestion (design.md, Connection Health Line) —
/// the progress bar truncates *before* the middle slot drops. The bar
/// used to reserve everything but two cells, so on a terminal narrower
/// than ~`health_width + 49` columns a Warning suggestion (and the
/// whole marquee) was invisible whenever a file was playing.
#[test]
fn progress_bar_truncates_before_the_suggestion_drops() {
    use dessplay::ui::props::{HealthProps, HealthSample, LinkStatus, SuggestionProps, Tone};
    use dessplay_core::types::PlaybackPosition;

    // A playing file: now-playing with a duration and our own position
    // sample, so the 47-cell progress text renders.
    let mut state = CrdtState::new();
    state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
    state.set_now_playing(A, ts(2), Some(hash(1)));
    state.set_playback_position(
        A,
        ts(3),
        UserId::new("kim"),
        PlaybackPosition {
            position_millis: 754_000,
            timestamp: ts(3),
            file: hash(1),
        },
    );
    let mut snap = snapshot(state.view(), vec![peer("kim")]);
    snap.health = HealthProps {
        link: LinkStatus::Connected,
        sample: Some(HealthSample {
            rtt_millis: Some(89),
            unanswered_probes: 0,
            server_silence_millis: 0,
            up_bps: 1_200_000,
            down_bps: 340_000,
        }),
        suggestion: Some(SuggestionProps {
            text: "high latency — disable BitTorrent (F3)".into(),
            tone: Tone::Paused,
        }),
        ..Default::default()
    };
    let mut ui = ui();
    ui.apply_snapshot(snap);

    // 80 columns: full progress (47) + health (32) leave 1 cell — the
    // bar must yield, not the warning.
    let rendered = render(&mut ui, 80, 30);
    let row = rendered
        .lines()
        .find(|line| line.contains("sync ok"))
        .expect("health row rendered");
    assert!(
        row.contains("high latency — disable BitTorrent (F3)"),
        "the warning renders; the bar truncates first: {row:?}"
    );
    assert!(
        row.starts_with('['),
        "the truncated progress bar is still drawn at the left: {row:?}"
    );
    assert!(
        !row.contains("12:34"),
        "the progress bar visibly truncated to make room: {row:?}"
    );
    assert!(row.ends_with("sync ok"), "health keeps its width: {row:?}");

    // A roomy terminal still shows everything untruncated — no
    // reservation shrinks the bar when there is space for both.
    let wide = render(&mut ui, 200, 30);
    let row = wide
        .lines()
        .find(|line| line.contains("sync ok"))
        .expect("health row rendered");
    assert!(row.contains("] 12:34 / 24:00"), "{row:?}");
    assert!(row.contains("disable BitTorrent (F3)"), "{row:?}");
}

/// The health row is dead to the mouse: a click there focuses nothing
/// and selects nothing (it is outside every recorded pane rect).
#[test]
fn click_on_health_row_changes_nothing() {
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    render(&mut ui, 100, 30);
    let (_, _, _, playlist) = pane_rects(100, 30);
    let health_row = playlist.y + playlist.height;

    // Chat is focused (the default); a click on the health row must not
    // move focus to the playlist above it.
    assert!(ui.handle(click(playlist.x + 5, health_row)).is_empty());
    let bar = render(&mut ui, 100, 30);
    assert!(
        bar.contains("Send") && !bar.contains("Remove"),
        "focus stayed on chat: {bar}"
    );
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
    assert!(screen.contains("Color overflow"), "{screen}");
    assert!(screen.contains("Reuse colors"), "{screen}");
    insta::assert_snapshot!(screen);
    ui.handle(key(Key::Esc));
    assert!(!ui.modal_open(), "Esc should close the settings modal");
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
fn settings_commentary_layout_snapshot() {
    let mut ui = ui();
    ui.handle(key(Key::Function(3)));
    for _ in 0..4 {
        ui.handle(key(Key::Right));
    }
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Settings — AI commentary"), "{screen}");
    assert!(screen.contains("Anthropic API token"), "{screen}");
    assert!(screen.contains("Baughn only"), "{screen}");
    assert!(
        screen.contains("Sends recent subtitles and a player screenshot to Anthropic."),
        "{screen}"
    );
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
    assert!(!ui.modal_open(), "saving should close the settings modal");
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
    for command in dessplay::ui::commands::SLASH_COMMANDS {
        assert!(
            all.contains(command.name),
            "{} missing: {all}",
            command.name
        );
    }
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

/// Shift-Tab as crossterm reports it: `BackTab` with the SHIFT modifier
/// set (some terminals send it with no modifier; both are accepted).
fn back_tab() -> Event<NoUserEvent> {
    Event::Keyboard(KeyEvent {
        code: Key::BackTab,
        modifiers: KeyModifiers::SHIFT,
    })
}

#[test]
fn shift_tab_cycles_focus_in_reverse() {
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    assert!(render(&mut ui, 100, 30).contains("Send"));

    // Chat -> Playlist (wraps backwards).
    ui.handle(back_tab());
    let playlist_bar = render(&mut ui, 100, 30);
    assert!(playlist_bar.contains("Remove"), "{playlist_bar}");

    // Playlist -> Users.
    ui.handle(back_tab());
    let users_bar = render(&mut ui, 100, 30);
    assert!(users_bar.contains("Mark away"), "{users_bar}");

    // Users -> Series -> Chat.
    ui.handle(back_tab());
    ui.handle(back_tab());
    assert!(render(&mut ui, 100, 30).contains("Send"));

    // Modifier-less BackTab is the same key.
    ui.handle(key(Key::BackTab));
    assert!(render(&mut ui, 100, 30).contains("Remove"));

    // Tab and Shift-Tab are inverses from every pane (both orders).
    for _ in 0..4 {
        let before = render(&mut ui, 100, 30);
        ui.handle(key(Key::Tab));
        ui.handle(back_tab());
        assert_eq!(render(&mut ui, 100, 30), before);
        ui.handle(back_tab());
        ui.handle(key(Key::Tab));
        assert_eq!(render(&mut ui, 100, 30), before);
        ui.handle(key(Key::Tab));
    }
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

/// A left-click focuses the pane under the pointer *and* selects the
/// clicked row (design.md, Mouse support): clicking playlist row 2 then
/// pressing `d` removes exactly that entry — proof of both the focus
/// change and the row selection in one observable action.
#[test]
fn click_focuses_pane_and_selects_the_clicked_row() {
    let mut state = CrdtState::new();
    state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
    state.push_playlist_entry(A, ts(2), entry(2, "ep2.mkv"));
    state.push_playlist_entry(A, ts(3), entry(3, "ep3.mkv"));
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));
    // The first draw records the pane rects; clicks before it miss.
    render(&mut ui, 100, 30);
    let (_, _, _, playlist) = pane_rects(100, 30);

    // Nothing is playing, so the unfocused playlist viewport starts at
    // the top: body row 1 is the second entry.
    assert!(
        ui.handle(click(playlist.x + 2, playlist.y + 2)).is_empty(),
        "a click selects locally, producing no outward actions"
    );
    let bar = render(&mut ui, 100, 30);
    assert!(bar.contains("Remove"), "focus moved to Playlist: {bar}");
    assert_eq!(
        ui.handle(key(Key::Char('d'))),
        vec![UserAction::Mutate(Mutation::RemovePlaylist {
            hash: hash(2)
        })]
    );
}

/// Clicking back into the chat column returns focus there (the keybar
/// follows, exactly like Tab).
#[test]
fn click_moves_focus_back_to_chat() {
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    render(&mut ui, 100, 30);
    let (chat, _, _, playlist) = pane_rects(100, 30);

    ui.handle(click(playlist.x + 2, playlist.y + 1));
    assert!(render(&mut ui, 100, 30).contains("Remove"));
    ui.handle(click(chat.x + 2, chat.y + 2));
    assert!(render(&mut ui, 100, 30).contains("Send"));
}

/// While a modal is open it captures all input; mouse events are
/// ignored rather than reaching the panes underneath.
#[test]
fn mouse_is_ignored_while_a_modal_is_open() {
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    render(&mut ui, 100, 30);
    let (_, _, _, playlist) = pane_rects(100, 30);

    ui.handle(key(Key::Function(3))); // settings modal
    assert!(ui.modal_open());
    assert!(ui.handle(click(playlist.x + 2, playlist.y + 1)).is_empty());
    assert!(
        ui.handle(wheel(playlist.x + 2, playlist.y + 1, false))
            .is_empty()
    );
    assert!(ui.modal_open());
    // Focus is untouched: closing the modal lands back in Chat.
    ui.handle(key(Key::Esc));
    assert!(render(&mut ui, 100, 30).contains("Send"));
}

/// The wheel only scrolls the pane under the pointer when that pane is
/// already focused: an unfocused pane neither scrolls (its cursor is
/// invisible, so the movement would be silent) nor steals focus
/// (touchpads emit wheel events by accident).
#[test]
fn wheel_scrolls_only_the_focused_pane() {
    let mut state = CrdtState::new();
    state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
    state.push_playlist_entry(A, ts(2), entry(2, "ep2.mkv"));
    state.push_playlist_entry(A, ts(3), entry(3, "ep3.mkv"));
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));
    render(&mut ui, 100, 30);
    let (_, _, _, playlist) = pane_rects(100, 30);

    // Chat is focused: wheel ticks over the playlist are ignored.
    ui.handle(wheel(playlist.x + 2, playlist.y + 1, false));
    ui.handle(wheel(playlist.x + 2, playlist.y + 1, false));
    assert!(
        render(&mut ui, 100, 30).contains("Send"),
        "focus stays on Chat"
    );
    for _ in 0..3 {
        ui.handle(key(Key::Tab)); // Chat -> Series -> Users -> Playlist
    }
    assert_eq!(
        ui.handle(key(Key::Char('d'))),
        vec![UserAction::Mutate(Mutation::RemovePlaylist {
            hash: hash(1)
        })],
        "the ignored wheel ticks left the cursor on the first row"
    );

    // Focused, the same wheel ticks move the selection.
    ui.handle(wheel(playlist.x + 2, playlist.y + 1, false));
    ui.handle(wheel(playlist.x + 2, playlist.y + 1, false));
    assert_eq!(
        ui.handle(key(Key::Char('d'))),
        vec![UserAction::Mutate(Mutation::RemovePlaylist {
            hash: hash(3)
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
    // Canonicalized: macOS $TMPDIR is a symlink into /private/var and the
    // add/map boundary canonicalizes.
    let dir_path = dir.path().canonicalize().unwrap();
    std::fs::write(dir_path.join("ep1.mkv"), b"video bytes").unwrap();
    let mut ui = Ui::new(
        UserId::new("kim"),
        Settings {
            username: Some("kim".into()),
            password: Some("x".into()),
            ..Settings::default()
        },
        vec![dir_path.to_path_buf()],
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
            path: dir_path.join("ep1.mkv"),
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
    // Canonicalized: macOS $TMPDIR is a symlink into /private/var and the
    // add/map boundary canonicalizes.
    let dir_path = dir.path().canonicalize().unwrap();
    std::fs::write(dir_path.join("ep1.mkv"), b"video bytes").unwrap();
    let mut ui = Ui::new(
        UserId::new("kim"),
        Settings {
            username: Some("kim".into()),
            password: Some("x".into()),
            ..Settings::default()
        },
        vec![dir_path.to_path_buf()],
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
    // Canonicalized: macOS $TMPDIR is a symlink into /private/var and the
    // add/map boundary canonicalizes.
    let dir_path = dir.path().canonicalize().unwrap();
    let file = dir_path.join("ep1.mkv");
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

/// Phase 31: a pasted existing-file path adds regardless of the focused
/// pane — a drag lands wherever the cursor happens to be, and there is
/// no use for posting a file *path* to chat. Here Chat (the default
/// focus) is focused, and the add still fires.
#[test]
fn paste_existing_file_path_adds_from_any_pane() {
    let dir = tempfile::tempdir().unwrap();
    // Canonicalized: macOS $TMPDIR is a symlink into /private/var and the
    // add/map boundary canonicalizes.
    let dir_path = dir.path().canonicalize().unwrap();
    let file = dir_path.join("ep1.mkv");
    std::fs::write(&file, b"video bytes").unwrap();
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
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

/// Phase 31: the drag forms terminals actually produce — backslash
/// escapes, quotes, `file://` URLs — normalize before the existing-file
/// test, so a dragged path with spaces adds instead of landing in chat.
#[test]
fn paste_escaped_path_normalizes_and_adds() {
    let dir = tempfile::tempdir().unwrap();
    // Canonicalized: macOS $TMPDIR is a symlink into /private/var and the
    // add/map boundary canonicalizes.
    let dir_path = dir.path().canonicalize().unwrap();
    let file = dir_path.join("ep 1.mkv");
    std::fs::write(&file, b"video bytes").unwrap();
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    let escaped = file.display().to_string().replace(' ', "\\ ");
    let actions = ui.handle(paste(&escaped));
    assert_eq!(
        actions,
        vec![UserAction::HashAndAdd {
            path: file,
            after: None,
        }]
    );
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
    // Canonicalized: macOS $TMPDIR is a symlink into /private/var and the
    // add/map boundary canonicalizes.
    let dir_path = dir.path().canonicalize().unwrap();
    let root = dir_path.join("Anime");
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
    // Canonicalized: macOS $TMPDIR is a symlink into /private/var and the
    // add/map boundary canonicalizes.
    let dir_path = dir.path().canonicalize().unwrap();
    let root = dir_path.join("Anime");
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
    // Canonicalized: macOS $TMPDIR is a symlink into /private/var and the
    // add/map boundary canonicalizes.
    let dir_path = dir.path().canonicalize().unwrap();
    for name in ["a-completely-unrelated-movie.avi", "ep2.mkv"] {
        std::fs::write(dir_path.join(name), b"x").unwrap();
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
        vec![dir_path.to_path_buf()],
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
            path: dir_path.join("ep2.mkv"),
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

/// Phase 33: `n` on a List entry opens the minimal Nero-name editor — a
/// rename in two keystrokes, without the full edit modal. Enter saves
/// (trimmed, empty clears), Esc cancels without a write, and committing
/// an unchanged value writes nothing.
#[test]
fn n_edits_nero_name_in_two_keystrokes() {
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
            watchers: Default::default(),
            anidb_series_id: None,
            local_aliases: Default::default(),
            manual_files: Default::default(),
            anidb_unavailable: false,
        },
    );
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim")]));
    ui.handle(key(Key::Tab)); // Series pane (The List)
    ui.handle(key(Key::Down)); // heading -> entry

    // Rename: the editor opens prefilled with the current nero_name.
    ui.handle(key(Key::Char('n')));
    assert!(ui.modal_open());
    for _ in 0.."Funeral".len() {
        ui.handle(key(Key::Backspace));
    }
    type_str(&mut ui, "  Sousou no Baughn ");
    let actions = ui.handle(key(Key::Enter));
    let [UserAction::Mutate(Mutation::PutListEntry { id, entry })] = actions.as_slice() else {
        panic!("expected PutListEntry, got {actions:?}");
    };
    assert_eq!(*id, ListEntryId(7));
    assert_eq!(entry.nero_name.as_deref(), Some("Sousou no Baughn"));
    assert_eq!(entry.name, "Frieren", "only nero_name changes");
    assert!(!ui.modal_open());

    // Esc cancels without a write.
    ui.handle(key(Key::Char('n')));
    type_str(&mut ui, "junk");
    assert!(ui.handle(key(Key::Esc)).is_empty());
    assert!(!ui.modal_open());

    // Committing the unchanged prefill writes nothing.
    ui.handle(key(Key::Char('n')));
    assert!(ui.handle(key(Key::Enter)).is_empty());
    assert!(!ui.modal_open());

    // Clearing the field clears the name.
    ui.handle(key(Key::Char('n')));
    for _ in 0.."Funeral".len() {
        ui.handle(key(Key::Backspace));
    }
    let actions = ui.handle(key(Key::Enter));
    let [UserAction::Mutate(Mutation::PutListEntry { entry, .. })] = actions.as_slice() else {
        panic!("expected PutListEntry, got {actions:?}");
    };
    assert_eq!(entry.nero_name, None);
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

// ---- Spoiler tags (design.md, Chat) ------------------------------------

fn snapshot_at(view: StateView, peers: Vec<PeerInfo>, now: u64) -> UiSnapshot {
    let mut snapshot = snapshot(view, peers);
    snapshot.now = now;
    snapshot.shared_now = now;
    snapshot
}

fn chat_message(state: &mut CrdtState, t: u64, from: &str, text: &str) {
    state.append_chat(dessplay_core::types::ChatMessage {
        timestamp: ts(t),
        sender: UserId::new(from),
        text: text.into(),
    });
}

/// The first chat message renders at the top of the log; its body
/// starts after the "HH:MM " (6) + "nero: " prefix and the left border.
/// "the ||secret|| twist" puts the run at display chars 4..10, so a
/// click at column 18 lands inside it on body row 1.
const SPOILER_CLICK: (u16, u16) = (18, 1);

#[test]
fn spoiler_scrambles_then_click_twice_reveals() {
    let mut state = CrdtState::new();
    chat_message(&mut state, 1_000, "nero", "the ||secret|| twist");
    let mut ui = ui();
    let peers = vec![peer("kim"), peer("nero")];
    // The shell freshens the animator clock before every input.
    ui.advance_clock(10_000);
    ui.apply_snapshot(snapshot_at(state.view(), peers.clone(), 10_000));
    let before = render(&mut ui, 100, 30);
    assert!(!before.contains("secret"), "spoiler leaked:\n{before}");
    assert!(!before.contains("||"), "bars leaked:\n{before}");
    assert!(before.contains("nero:"), "{before}");
    // The low-grade zalgo made it to the buffer: combining marks ride
    // on the scrambled letters.
    assert!(
        before
            .chars()
            .any(|c| ('\u{0300}'..='\u{0330}').contains(&c)),
        "no combining marks in:\n{before}"
    );

    // First click: the re-randomization tease starts.
    let (col, row) = SPOILER_CLICK;
    ui.handle(click(col, row));
    // Frames derive from the clock; the shell's pre-input freshen
    // carries it forward at snapshot rate.
    ui.advance_clock(10_300);
    ui.apply_snapshot(snapshot_at(state.view(), peers.clone(), 10_300));
    let teasing = render(&mut ui, 100, 30);
    assert!(!teasing.contains("secret"), "{teasing}");
    assert_ne!(teasing, before, "the tease should change the letters");

    // Second click within 5s reveals for the session.
    ui.handle(click(col, row));
    let revealed = render(&mut ui, 100, 30);
    assert!(revealed.contains("the secret twist"), "{revealed}");
    assert!(!revealed.contains("||"), "{revealed}");
}

#[test]
fn lapsed_reveal_window_requires_a_fresh_double_click() {
    let mut state = CrdtState::new();
    chat_message(&mut state, 1_000, "nero", "the ||secret|| twist");
    let mut ui = ui();
    let peers = vec![peer("kim"), peer("nero")];
    // The shell freshens the animator clock before every input.
    ui.advance_clock(10_000);
    ui.apply_snapshot(snapshot_at(state.view(), peers.clone(), 10_000));
    render(&mut ui, 100, 30);
    let (col, row) = SPOILER_CLICK;
    ui.handle(click(col, row));
    // The tease finishes and the 5s window lapses.
    ui.advance_clock(20_000);
    ui.apply_snapshot(snapshot_at(state.view(), peers.clone(), 20_000));
    // This click is a fresh first click (re-tease), not a reveal…
    ui.handle(click(col, row));
    let still_hidden = render(&mut ui, 100, 30);
    assert!(!still_hidden.contains("secret"), "{still_hidden}");
    // …and the next one (within the new window) reveals.
    ui.handle(click(col, row));
    assert!(render(&mut ui, 100, 30).contains("the secret twist"));
}

#[test]
fn own_spoilers_are_scrambled_too() {
    let mut state = CrdtState::new();
    chat_message(&mut state, 1_000, "kim", "mine ||hidden|| words");
    let mut ui = ui();
    ui.apply_snapshot(snapshot_at(state.view(), vec![peer("kim")], 10_000));
    let screen = render(&mut ui, 100, 30);
    assert!(!screen.contains("hidden"), "own spoiler leaked:\n{screen}");
    assert!(!screen.contains("||"), "{screen}");
}

#[test]
fn reveal_command_walks_newest_to_oldest_then_notices() {
    let mut state = CrdtState::new();
    chat_message(&mut state, 1_000, "nero", "first ||alpha|| here");
    chat_message(&mut state, 2_000, "nero", "second ||beta|| here");
    let mut ui = ui();
    ui.apply_snapshot(snapshot_at(
        state.view(),
        vec![peer("kim"), peer("nero")],
        10_000,
    ));
    render(&mut ui, 100, 30);

    // Newest first.
    type_str(&mut ui, "/reveal");
    assert!(ui.handle(key(Key::Enter)).is_empty());
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("beta"), "{screen}");
    assert!(!screen.contains("alpha"), "{screen}");

    // Repeat reveals the older one.
    type_str(&mut ui, "/reveal");
    assert!(ui.handle(key(Key::Enter)).is_empty());
    assert!(render(&mut ui, 100, 30).contains("alpha"));

    // Nothing hidden left: a local notice, no state change.
    type_str(&mut ui, "/reveal");
    let actions = ui.handle(key(Key::Enter));
    assert!(
        matches!(actions.as_slice(), [UserAction::Notice(text)] if text.contains("no hidden spoiler")),
        "{actions:?}"
    );
}

#[test]
fn spoiler_reveal_state_survives_scrolling() {
    // The reveal is keyed by message identity, not screen position: a
    // scroll between the two clicks cannot retarget it.
    let mut state = CrdtState::new();
    for i in 0..24 {
        chat_message(&mut state, 1_000 + i, "nero", "filler line");
    }
    chat_message(&mut state, 2_000, "nero", "the ||secret|| twist");
    let mut ui = ui();
    ui.apply_snapshot(snapshot_at(
        state.view(),
        vec![peer("kim"), peer("nero")],
        10_000,
    ));
    let screen = render(&mut ui, 100, 30);
    assert!(!screen.contains("secret"), "{screen}");

    // The newest message sits on the last body row of the log. The left
    // column is panes-area height (30 - 3 status - 1 keybar - 1 bottom
    // line); the log is that minus the 3-row input, minus 2 border rows.
    let (left, ..) = pane_rects(100, 30);
    let log_bottom_row = left.y + (left.height - 3) - 2;
    ui.handle(click(18, log_bottom_row));
    // Scroll up and back down between the two clicks (chat has focus).
    ui.handle(wheel(10, 5, true));
    render(&mut ui, 100, 30);
    ui.handle(wheel(10, 5, false));
    render(&mut ui, 100, 30);
    ui.handle(click(18, log_bottom_row));
    assert!(
        render(&mut ui, 100, 30).contains("the secret twist"),
        "reveal lost across a scroll"
    );
}

// ---- Chat drag-selection copy (design.md, Mouse support) ---------------

fn drag_to(column: u16, row: u16) -> Event<NoUserEvent> {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        modifiers: KeyModifiers::NONE,
        column,
        row,
    })
}

fn release(column: u16, row: u16) -> Event<NoUserEvent> {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        modifiers: KeyModifiers::NONE,
        column,
        row,
    })
}

fn shift_key(code: Key) -> Event<NoUserEvent> {
    Event::Keyboard(KeyEvent {
        code,
        modifiers: KeyModifiers::SHIFT,
    })
}

/// The whole-lines copy timestamp, computed the same way the app does
/// (local timezone), so the expectation holds in any TZ.
fn hhmmss_local(millis: u64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_millis_opt(millis as i64)
        .single()
        .unwrap()
        .format("%H:%M:%S")
        .to_string()
}

/// Screen (column, row) of the first occurrence of `needle` in the
/// rendered buffer. Columns count chars, not bytes — the border glyphs
/// are multi-byte (single-width cells only in these tests).
fn locate(rendered: &str, needle: &str) -> (u16, u16) {
    for (row, line) in rendered.lines().enumerate() {
        if let Some(idx) = line.find(needle) {
            return (line[..idx].chars().count() as u16, row as u16);
        }
    }
    panic!("{needle:?} not on screen:\n{rendered}");
}

/// A Ui showing two chat messages from amu, plus their millis.
fn selection_ui() -> (Ui, u64, u64) {
    let (m1, m2) = (1_000_000, 1_060_000);
    let mut state = CrdtState::new();
    chat_message(&mut state, m1, "amu", "hello world");
    chat_message(&mut state, m2, "amu", "second line");
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim"), peer("amu")]));
    (ui, m1, m2)
}

/// Dragging within one message copies exactly the dragged cells,
/// verbatim, on release — and only on release.
#[test]
fn drag_within_one_message_copies_the_selected_text_verbatim() {
    let (mut ui, ..) = selection_ui();
    let rendered = render(&mut ui, 100, 30);
    let (col, row) = locate(&rendered, "world");

    assert!(ui.handle(click(col, row)).is_empty());
    assert!(
        ui.handle(drag_to(col + 4, row)).is_empty(),
        "no copy mid-drag"
    );
    assert_eq!(
        ui.handle(release(col + 4, row)),
        vec![UserAction::CopyToClipboard("world".into())]
    );
}

/// Dragging across two messages snaps to whole lines and copies them in
/// the irccloud log format ("HH:MM:SS <nick> text").
#[test]
fn drag_across_messages_copies_whole_lines_irccloud_style() {
    let (mut ui, m1, m2) = selection_ui();
    let rendered = render(&mut ui, 100, 30);
    let (col1, row1) = locate(&rendered, "hello world");
    let (_, row2) = locate(&rendered, "second line");

    ui.handle(click(col1 + 2, row1));
    ui.handle(drag_to(col1 + 5, row2));
    let expected = format!(
        "{} <amu> hello world\n{} <amu> second line",
        hhmmss_local(m1),
        hhmmss_local(m2)
    );
    assert_eq!(
        ui.handle(release(col1 + 5, row2)),
        vec![UserAction::CopyToClipboard(expected)]
    );
}

/// A motionless press-and-release is a click, not a selection: the
/// clipboard is never touched.
#[test]
fn plain_click_never_touches_the_clipboard() {
    let (mut ui, ..) = selection_ui();
    let rendered = render(&mut ui, 100, 30);
    let (col, row) = locate(&rendered, "world");

    assert!(ui.handle(click(col, row)).is_empty());
    assert!(ui.handle(release(col, row)).is_empty());
}

/// Shift-Down on a held partial selection widens it to whole lines
/// (from the start of the first selected line, gdocs-style) and
/// re-copies; Shift-Up steps the focus end back.
#[test]
fn shift_up_down_extend_a_held_selection_by_whole_lines() {
    let (mut ui, m1, m2) = selection_ui();
    let rendered = render(&mut ui, 100, 30);
    let (col, row) = locate(&rendered, "world");

    ui.handle(click(col, row));
    ui.handle(drag_to(col + 4, row));
    ui.handle(release(col + 4, row));

    let both = format!(
        "{} <amu> hello world\n{} <amu> second line",
        hhmmss_local(m1),
        hhmmss_local(m2)
    );
    assert_eq!(
        ui.handle(shift_key(Key::Down)),
        vec![UserAction::CopyToClipboard(both)]
    );
    let first = format!("{} <amu> hello world", hhmmss_local(m1));
    assert_eq!(
        ui.handle(shift_key(Key::Up)),
        vec![UserAction::CopyToClipboard(first)]
    );
}

/// Any unrelated key dismisses the held selection (and still does its
/// normal job); Shift-Down afterwards has nothing left to extend.
#[test]
fn unrelated_key_clears_the_held_selection() {
    let (mut ui, ..) = selection_ui();
    let rendered = render(&mut ui, 100, 30);
    let (col, row) = locate(&rendered, "world");

    ui.handle(click(col, row));
    ui.handle(drag_to(col + 4, row));
    ui.handle(release(col + 4, row));

    ui.handle(key(Key::Char('x')));
    assert!(
        ui.handle(shift_key(Key::Down)).is_empty(),
        "cleared selection must not extend or copy"
    );
}

/// A drag spanning the wrap boundary of one long message is still a
/// partial selection of that one message: the copy is the verbatim
/// text, unbroken by the visual wrap (no newline, no indent).
#[test]
fn drag_across_a_wrap_boundary_stays_within_the_message() {
    let body = "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima";
    let mut state = CrdtState::new();
    chat_message(&mut state, 1_000_000, "amu", body);
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim"), peer("amu")]));
    let rendered = render(&mut ui, 100, 30);
    let (col1, row1) = locate(&rendered, "bravo");
    let (col2, row2) = locate(&rendered, "golf");
    assert!(row2 > row1, "the message must wrap for this test");

    ui.handle(click(col1, row1));
    ui.handle(drag_to(col2 + 3, row2));
    let start = body.find("bravo").unwrap();
    let end = body.find("golf").unwrap() + "golf".len();
    assert_eq!(
        ui.handle(release(col2 + 3, row2)),
        vec![UserAction::CopyToClipboard(body[start..end].into())]
    );
}

/// A whole-lines copy spanning a day separator skips the separator: it
/// is a render-time divider, not a message.
#[test]
fn whole_line_copy_skips_day_separators() {
    let (m1, m2) = (1_000_000, 1_000_000 + 86_400_000);
    let mut state = CrdtState::new();
    chat_message(&mut state, m1, "amu", "first day");
    chat_message(&mut state, m2, "amu", "second day");
    let mut ui = ui();
    ui.apply_snapshot(snapshot(state.view(), vec![peer("kim"), peer("amu")]));
    let rendered = render(&mut ui, 100, 30);
    let (col1, row1) = locate(&rendered, "first day");
    let (_, row2) = locate(&rendered, "second day");
    assert!(
        row2 - row1 >= 2,
        "a separator row must sit between:\n{rendered}"
    );

    ui.handle(click(col1, row1));
    ui.handle(drag_to(col1, row2));
    let expected = format!(
        "{} <amu> first day\n{} <amu> second day",
        hhmmss_local(m1),
        hhmmss_local(m2)
    );
    assert_eq!(
        ui.handle(release(col1, row2)),
        vec![UserAction::CopyToClipboard(expected)]
    );
}

// ---- Changelog (design.md, Changelog) ----------------------------------

fn changelog_days() -> Vec<dessplay::changelog::ChangelogDay> {
    dessplay::changelog::parse(
        "## 2026-09-02\n- Added: a brand new changelog\n## 2026-09-01\n- Fixed: an old bug\n",
    )
    .unwrap()
}

/// The startup "What's new" modal sits on top, swallows pane keys (it
/// opens under the user's hands), shows its [ OK ] button, and Esc
/// dismisses it with the marker the opener supplied.
#[test]
fn whats_new_modal_swallows_keys_and_esc_persists_marker() {
    let days = changelog_days();
    let marker = dessplay::changelog::latest_marker(&days).unwrap();
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    ui.show_changelog(days, marker);

    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("What's new"), "modal missing:\n{screen}");
    assert!(screen.contains("a brand new changelog"), "{screen}");
    assert!(screen.contains("an old bug"), "{screen}");
    // The dismiss affordance is visible, not guessed.
    assert!(screen.contains("[ OK ]"), "OK button missing:\n{screen}");

    // Pane keys and typing must not leak through or answer the modal.
    assert_eq!(ui.handle(key(Key::Char('w'))), vec![]);
    assert_eq!(ui.handle(key(Key::Tab)), vec![]);

    // Esc closes it and persists how far the user has read.
    assert_eq!(
        ui.handle(key(Key::Esc)),
        vec![UserAction::ChangelogSeen { marker }]
    );
    let screen = render(&mut ui, 100, 30);
    assert!(!screen.contains("What's new"), "modal stuck:\n{screen}");
}

/// `/changelog` opens the full-history viewer any time; Enter (the
/// [ OK ] button) closes it and re-persists the (identical) marker —
/// one path, no special cases.
#[test]
fn slash_changelog_opens_full_view() {
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    assert_eq!(type_str(&mut ui, "/changelog"), vec![]);
    assert_eq!(ui.handle(key(Key::Enter)), vec![]);

    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Changelog"), "modal missing:\n{screen}");
    // The embedded changelog's newest day is on screen.
    let newest = dessplay::changelog::entries()[0].date.to_string();
    assert!(screen.contains(&newest), "{screen}");

    let marker = dessplay::changelog::latest_marker(dessplay::changelog::entries()).unwrap();
    assert_eq!(
        ui.handle(key(Key::Enter)),
        vec![UserAction::ChangelogSeen { marker }]
    );
}

#[test]
fn f11_logs_leave_recent_chat_visible_and_restore_the_previous_modal() {
    let mut ui = ui();
    let (_subscriber, logs) =
        dessplay::logging::interactive_subscriber(tracing_subscriber::EnvFilter::new("info"), None);
    ui.set_logging(logs);
    ui.push_system(1000, "Recent chat remains visible".into());
    ui.handle(key(Key::Function(11)));
    let screen = render(&mut ui, 100, 30);
    assert!(screen.contains("Logs · LIVE"));
    assert!(screen.contains("Recent chat remains visible"));
    insta::assert_snapshot!("f11_log_view", screen);
    ui.handle(key(Key::Function(11)));
    assert!(!render(&mut ui, 100, 30).contains("Logs · LIVE"));

    ui.handle(key(Key::Function(3)));
    let settings = render(&mut ui, 100, 30);
    ui.handle(key(Key::Function(11)));
    assert!(render(&mut ui, 100, 30).contains("Logs · LIVE"));
    ui.handle(key(Key::Esc));
    assert_eq!(render(&mut ui, 100, 30), settings);
}

#[test]
fn log_dropdowns_apply_independently_cancel_and_survive_reopening() {
    use dessplay::logging::{LogLevel, interactive_subscriber};
    let (subscriber, logs) =
        interactive_subscriber(tracing_subscriber::EnvFilter::new("info"), None);
    tracing::subscriber::with_default(subscriber, || {
        let mut ui = ui();
        ui.set_logging(logs.clone());
        ui.handle(key(Key::Function(11)));
        ui.handle(key(Key::Tab)); // DessPlay
        ui.handle(key(Key::Enter));
        ui.handle(key(Key::End)); // trace
        ui.handle(key(Key::Esc)); // Cancel, without closing the modal.
        assert!(render(&mut ui, 100, 30).contains("Logs · LIVE"));
        assert_eq!(logs.levels(), [LogLevel::Startup; 2]);
        ui.handle(key(Key::Enter));
        ui.handle(key(Key::End));
        ui.handle(key(Key::Enter));
        assert_eq!(logs.levels(), [LogLevel::Trace, LogLevel::Startup]);
        ui.handle(key(Key::Tab)); // Rust
        ui.handle(key(Key::Enter));
        ui.handle(key(Key::Down)); // off
        ui.handle(key(Key::Enter));
        assert_eq!(logs.levels(), [LogLevel::Trace, LogLevel::Off]);
        tracing::trace!(target: "dessplay_core", "included application trace");
        tracing::error!(target: "quinn", "excluded dependency error");
        let screen = render(&mut ui, 100, 30);
        assert!(screen.contains("included application trace"));
        assert!(!screen.contains("excluded dependency error"));
        ui.handle(key(Key::Function(11)));
        ui.handle(key(Key::Function(11)));
        assert_eq!(logs.levels(), [LogLevel::Trace, LogLevel::Off]);
        assert!(render(&mut ui, 100, 30).contains("DessPlay: [ trace"));
    });
}

#[test]
fn log_scrollback_stays_anchored_during_appends_and_eviction_and_end_resumes_live() {
    let (subscriber, logs) =
        dessplay::logging::interactive_subscriber(tracing_subscriber::EnvFilter::new("info"), None);
    tracing::subscriber::with_default(subscriber, || {
        let mut ui = ui();
        ui.set_logging(logs);
        for n in 0..100 {
            tracing::info!("retained event {n:04}");
        }
        ui.handle(key(Key::Function(11)));
        assert!(render(&mut ui, 120, 30).contains("retained event 0099"));
        ui.handle(key(Key::Home));
        let before = render(&mut ui, 120, 30);
        assert!(before.contains("retained event 0000"));
        for n in 100..150 {
            tracing::info!("retained event {n:04}");
        }
        assert!(
            ui.advance_clock(2000),
            "idle log arrivals request a repaint"
        );
        assert_eq!(render(&mut ui, 120, 30), before);
        assert!(
            !ui.advance_clock(2000),
            "an unchanged log must not repaint forever"
        );
        for n in 150..2100 {
            tracing::info!("retained event {n:04}");
        }
        let evicted = render(&mut ui, 120, 30);
        assert!(evicted.contains("retained event 0100"));
        assert!(!evicted.contains("retained event 0000"));
        ui.handle(key(Key::End));
        assert!(render(&mut ui, 120, 30).contains("retained event 2099"));
        tracing::info!("a new live event");
        assert!(render(&mut ui, 120, 30).contains("a new live event"));
    });
}

#[test]
fn log_view_and_dropdowns_fit_small_terminals_and_capture_typing() {
    let (_subscriber, logs) =
        dessplay::logging::interactive_subscriber(tracing_subscriber::EnvFilter::new("info"), None);
    let mut ui = ui();
    ui.set_logging(logs);
    ui.handle(key(Key::Function(11)));
    assert!(type_str(&mut ui, "do not send to chat").is_empty());
    assert!(ui.handle(paste("do not paste to chat")).is_empty());
    ui.handle(key(Key::Tab));
    ui.handle(key(Key::Enter));
    for width in [1, 10, 25, 80] {
        for height in [1, 2, 5, 12, 24] {
            let _ = render(&mut ui, width, height);
        }
    }
    ui.handle(key(Key::Function(11)));
    let screen = render(&mut ui, 100, 30);
    assert!(!screen.contains("do not send"));
    assert!(!screen.contains("do not paste"));
}

#[test]
fn roguelike_captures_input_saves_before_advancing_and_restores_modals() {
    use dessplay::roguelike::{Action, Run};
    use dessplay::roguelike_store::Command;
    let mut ui = ui();
    type_str(&mut ui, "chat draft");
    ui.handle(key(Key::Function(3)));
    assert_eq!(
        ui.handle(key(Key::Function(4))),
        vec![UserAction::Roguelike(Command::Open)]
    );
    assert!(
        ui.handle(key(Key::Right)).is_empty(),
        "loading must capture movement"
    );
    ui.set_roguelike(Ok(Box::new(Run::new(42).view())));
    ui.push_system(1000, "Party chat stays visible".into());
    let screen = render(&mut ui, 100, 45);
    assert!(screen.contains("THE WAITING BELOW"));
    assert!(screen.contains("Party chat stays visible"));
    assert!(ui.handle(paste("...jjjj")).is_empty());
    assert_eq!(
        ui.handle(key(Key::Right)),
        vec![UserAction::Roguelike(Command::Act(Action::Move(1, 0)))]
    );
    assert!(
        ui.handle(key(Key::Right)).is_empty(),
        "wait for durable acknowledgement"
    );
    ui.set_roguelike(Err("Disk full: previous turn retained".into()));
    assert!(render(&mut ui, 100, 45).contains("Disk full"));
    ui.set_roguelike(Ok(Box::new(Run::new(42).view())));
    ui.handle(key(Key::Function(11)));
    assert!(render(&mut ui, 100, 45).contains("Logs"));
    ui.handle(key(Key::Esc));
    assert!(render(&mut ui, 100, 45).contains("THE WAITING BELOW"));
    ui.handle(key(Key::Esc));
    assert!(render(&mut ui, 100, 45).contains("Settings"));
    ui.handle(key(Key::Esc));
    assert!(render(&mut ui, 100, 45).contains("chat draft"));
}

#[test]
fn roguelike_arrivals_are_sticky_and_include_returns_but_exclude_seeders_and_self() {
    use dessplay::roguelike::Run;
    let mut ui = ui();
    ui.apply_snapshot(snapshot(StateView::default(), vec![peer("kim")]));
    ui.handle(key(Key::Function(4)));
    ui.set_roguelike(Ok(Box::new(Run::new(42).view())));
    let mut seeder = peer("warehouse");
    seeder.role = Role::Seeder;
    ui.apply_snapshot(snapshot(
        StateView::default(),
        vec![peer("kim"), seeder.clone()],
    ));
    assert!(!render(&mut ui, 100, 45).contains("warehouse joined"));
    // The game remains updated even under logs.
    ui.handle(key(Key::Function(11)));
    let arrived = vec![peer("kim"), seeder, peer("Nero")];
    ui.apply_snapshot(snapshot(StateView::default(), arrived.clone()));
    ui.handle(key(Key::Esc));
    assert!(render(&mut ui, 100, 45).contains("Nero joined"));
    ui.advance_clock(1_000_000);
    assert!(render(&mut ui, 100, 45).contains("Nero joined"));
    ui.handle(key(Key::Enter));
    ui.apply_snapshot(snapshot(StateView::default(), arrived.clone()));
    assert!(!render(&mut ui, 100, 45).contains("Nero joined"));
    let mut lost = arrived.clone();
    lost[2].presence = Presence::Lost;
    ui.apply_snapshot(snapshot(StateView::default(), lost));
    ui.apply_snapshot(snapshot(StateView::default(), arrived));
    assert!(render(&mut ui, 100, 45).contains("Nero joined"));
}

#[test]
fn roguelike_slash_command_and_tiny_terminals() {
    let mut ui = ui();
    type_str(&mut ui, "/rogue");
    assert_eq!(
        ui.handle(key(Key::Enter)),
        vec![UserAction::Roguelike(
            dessplay::roguelike_store::Command::Open
        )]
    );
    ui.set_roguelike(Ok(Box::new(dessplay::roguelike::Run::new(1).view())));
    for (width, height) in [(1, 1), (10, 4), (40, 12), (80, 24), (120, 50)] {
        render(&mut ui, width, height);
    }
    ui.handle(key(Key::Char('?')));
    assert!(render(&mut ui, 100, 45).contains("THE WAITING BELOW"));
}
