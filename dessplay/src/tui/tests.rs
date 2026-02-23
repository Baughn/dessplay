#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod snapshot_tests {
    use insta::assert_snapshot;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use crate::tui::layout::compute_layout;
    use crate::tui::ui_state::{FocusedPane, InputState};
    use crate::tui::widgets::{
        chat, keybinding_bar, player_status, playlist, recent_series, users,
    };
    use dessplay_core::types::{FileId, FileState, UserState};

    fn fid(n: u8) -> FileId {
        let mut id = [0u8; 16];
        id[0] = n;
        FileId(id)
    }

    // -----------------------------------------------------------------------
    // Main layout tests
    // -----------------------------------------------------------------------

    #[test]
    fn main_layout_120x40() {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|frame| {
            let area = frame.area();
            let layout = compute_layout(area);

            let msgs: Vec<(&dessplay_core::types::UserId, &str)> = vec![];
            let input = InputState::new();
            chat::render_chat_messages(
                layout.chat_messages, frame.buffer_mut(), &msgs, 0, true,
            );
            chat::render_chat_input(
                layout.chat_input, frame.buffer_mut(), &input, true,
            );
            recent_series::render_recent_series(
                layout.recent_series, frame.buffer_mut(), &[], 0, false,
            );
            users::render_users(
                layout.users, frame.buffer_mut(), &[], false,
            );
            playlist::render_playlist(
                layout.playlist, frame.buffer_mut(), &[], 0, false,
            );
            player_status::render_player_status(
                layout.player_status, frame.buffer_mut(), None, 0.0, None, false, &[],
            );
            keybinding_bar::render_keybinding_bar(
                layout.keybinding_bar, frame.buffer_mut(),
                &FocusedPane::Chat,
            );
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    #[test]
    fn main_layout_80x24() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| {
            let area = frame.area();
            let layout = compute_layout(area);

            let msgs: Vec<(&dessplay_core::types::UserId, &str)> = vec![];
            let input = InputState::new();
            chat::render_chat_messages(
                layout.chat_messages, frame.buffer_mut(), &msgs, 0, true,
            );
            chat::render_chat_input(
                layout.chat_input, frame.buffer_mut(), &input, true,
            );
            recent_series::render_recent_series(
                layout.recent_series, frame.buffer_mut(), &[], 0, false,
            );
            users::render_users(
                layout.users, frame.buffer_mut(), &[], false,
            );
            playlist::render_playlist(
                layout.playlist, frame.buffer_mut(), &[], 0, false,
            );
            player_status::render_player_status(
                layout.player_status, frame.buffer_mut(), None, 0.0, None, false, &[],
            );
            keybinding_bar::render_keybinding_bar(
                layout.keybinding_bar, frame.buffer_mut(),
                &FocusedPane::Chat,
            );
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    // -----------------------------------------------------------------------
    // Chat widget tests
    // -----------------------------------------------------------------------

    #[test]
    fn chat_with_messages() {
        let alice = dessplay_core::types::UserId("alice".to_string());
        let bob = dessplay_core::types::UserId("bob".to_string());
        let msgs: Vec<(&dessplay_core::types::UserId, &str)> = vec![
            (&alice, "hello everyone!"),
            (&bob, "hey alice, how's it going?"),
            (&alice, "good! ready to watch?"),
        ];

        let mut terminal = Terminal::new(TestBackend::new(50, 12)).unwrap();
        terminal.draw(|frame| {
            let inner = ratatui::layout::Rect {
                x: 0, y: 0, width: 50, height: 11,
            };
            chat::render_chat_messages(inner, frame.buffer_mut(), &msgs, 0, true);
            let input_area = ratatui::layout::Rect {
                x: 0, y: 11, width: 50, height: 1,
            };
            let mut input = InputState::new();
            for c in "typing something".chars() {
                input.insert_char(c);
            }
            chat::render_chat_input(input_area, frame.buffer_mut(), &input, true);
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    #[test]
    fn chat_empty() {
        let msgs: Vec<(&dessplay_core::types::UserId, &str)> = vec![];
        let mut terminal = Terminal::new(TestBackend::new(50, 10)).unwrap();
        terminal.draw(|frame| {
            chat::render_chat_messages(
                frame.area(), frame.buffer_mut(), &msgs, 0, false,
            );
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    // -----------------------------------------------------------------------
    // Users widget tests
    // -----------------------------------------------------------------------

    #[test]
    fn users_all_ready() {
        let entries = vec![
            users::UserEntry {
                name: "alice".to_string(),
                user_state: UserState::Ready,
                file_state: FileState::Ready,
                is_self: true,
            },
            users::UserEntry {
                name: "bob".to_string(),
                user_state: UserState::Ready,
                file_state: FileState::Ready,
                is_self: false,
            },
        ];

        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        terminal.draw(|frame| {
            users::render_users(frame.area(), frame.buffer_mut(), &entries, false);
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    #[test]
    fn users_mixed_states() {
        let entries = vec![
            users::UserEntry {
                name: "alice".to_string(),
                user_state: UserState::Ready,
                file_state: FileState::Ready,
                is_self: true,
            },
            users::UserEntry {
                name: "bob".to_string(),
                user_state: UserState::Paused,
                file_state: FileState::Ready,
                is_self: false,
            },
            users::UserEntry {
                name: "charlie".to_string(),
                user_state: UserState::NotWatching,
                file_state: FileState::Ready,
                is_self: false,
            },
            users::UserEntry {
                name: "dave".to_string(),
                user_state: UserState::Ready,
                file_state: FileState::Downloading { progress: 0.15 },
                is_self: false,
            },
        ];

        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal.draw(|frame| {
            users::render_users(frame.area(), frame.buffer_mut(), &entries, false);
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    #[test]
    fn users_no_peers() {
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
        terminal.draw(|frame| {
            users::render_users(frame.area(), frame.buffer_mut(), &[], false);
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    // -----------------------------------------------------------------------
    // Playlist widget tests
    // -----------------------------------------------------------------------

    #[test]
    fn playlist_with_items() {
        let entries = vec![
            playlist::PlaylistEntry {
                file_id: fid(1),
                display_name: "Frieren - 01.mkv".to_string(),
                is_missing: false,
                is_current: true,
            },
            playlist::PlaylistEntry {
                file_id: fid(2),
                display_name: "Frieren - 02.mkv".to_string(),
                is_missing: false,
                is_current: false,
            },
            playlist::PlaylistEntry {
                file_id: fid(3),
                display_name: "Frieren - 03.mkv".to_string(),
                is_missing: true,
                is_current: false,
            },
        ];

        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal.draw(|frame| {
            playlist::render_playlist(
                frame.area(), frame.buffer_mut(), &entries, 1, true,
            );
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    #[test]
    fn playlist_empty() {
        let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
        terminal.draw(|frame| {
            playlist::render_playlist(
                frame.area(), frame.buffer_mut(), &[], 0, false,
            );
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    // -----------------------------------------------------------------------
    // Keybinding bar tests
    // -----------------------------------------------------------------------

    #[test]
    fn keybinding_bar_chat() {
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).unwrap();
        terminal.draw(|frame| {
            keybinding_bar::render_keybinding_bar(
                frame.area(), frame.buffer_mut(),
                &FocusedPane::Chat,
            );
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    #[test]
    fn keybinding_bar_playlist() {
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).unwrap();
        terminal.draw(|frame| {
            keybinding_bar::render_keybinding_bar(
                frame.area(), frame.buffer_mut(),
                &FocusedPane::Playlist,
            );
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    #[test]
    fn keybinding_bar_recent_series() {
        let mut terminal = Terminal::new(TestBackend::new(80, 1)).unwrap();
        terminal.draw(|frame| {
            keybinding_bar::render_keybinding_bar(
                frame.area(), frame.buffer_mut(),
                &FocusedPane::RecentSeries,
            );
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    // -----------------------------------------------------------------------
    // Player status tests
    // -----------------------------------------------------------------------

    #[test]
    fn player_status_idle() {
        let mut terminal = Terminal::new(TestBackend::new(60, 3)).unwrap();
        terminal.draw(|frame| {
            player_status::render_player_status(
                frame.area(), frame.buffer_mut(), None, 0.0, None, false, &[],
            );
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    #[test]
    fn player_status_playing() {
        let mut terminal = Terminal::new(TestBackend::new(60, 3)).unwrap();
        terminal.draw(|frame| {
            player_status::render_player_status(
                frame.area(), frame.buffer_mut(), Some("Frieren - 01.mkv"),
                754.0, Some(1440.0), true, &[],
            );
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    // -----------------------------------------------------------------------
    // Settings widget tests
    // -----------------------------------------------------------------------

    #[test]
    fn settings_all_valid() {
        use crate::tui::ui_state::SettingsState;
        use crate::tui::widgets::settings;

        let mut state = SettingsState::new();
        state.username = "alice".to_string();
        state.server = "dessplay.brage.info:4433".to_string();
        state.player = "mpv".to_string();
        state.password = "secret".to_string();
        state.media_roots = vec![
            std::path::PathBuf::from("/home/alice/anime"),
        ];

        let mut terminal = Terminal::new(TestBackend::new(70, 20)).unwrap();
        terminal.draw(|frame| {
            let wf = keybinding_bar::WindowFrame::new(frame.area());
            settings::render_settings(wf.content, frame.buffer_mut(), &state);
            wf.render_bar(frame.buffer_mut(), settings::keybindings());
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    #[test]
    fn settings_invalid_server() {
        use crate::tui::ui_state::SettingsState;
        use crate::tui::widgets::settings;

        let mut state = SettingsState::new();
        state.username = "alice".to_string();
        state.server = "localhost".to_string(); // missing port
        state.player = "mpv".to_string();
        state.media_roots = vec![
            std::path::PathBuf::from("/home/alice/anime"),
        ];

        let mut terminal = Terminal::new(TestBackend::new(70, 20)).unwrap();
        terminal.draw(|frame| {
            let wf = keybinding_bar::WindowFrame::new(frame.area());
            settings::render_settings(wf.content, frame.buffer_mut(), &state);
            wf.render_bar(frame.buffer_mut(), settings::keybindings());
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    #[test]
    fn settings_with_alert() {
        use crate::tui::ui_state::SettingsState;
        use crate::tui::widgets::settings;

        let mut state = SettingsState::new();
        state.username = "alice".to_string();
        state.server = "dessplay.brage.info:4433".to_string();
        state.player = "mpv".to_string();
        state.media_roots = vec![
            std::path::PathBuf::from("/home/alice/anime"),
        ];
        state.alert = Some("failed to resolve server address: dessplay.brage.info:4433".to_string());

        let mut terminal = Terminal::new(TestBackend::new(70, 20)).unwrap();
        terminal.draw(|frame| {
            let wf = keybinding_bar::WindowFrame::new(frame.area());
            settings::render_settings(wf.content, frame.buffer_mut(), &state);
            wf.render_bar(frame.buffer_mut(), settings::keybindings());
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }

    #[test]
    fn settings_all_empty() {
        use crate::tui::ui_state::SettingsState;
        use crate::tui::widgets::settings;

        let mut state = SettingsState::new();
        state.username = String::new();
        state.server = String::new();
        state.player = "mpv".to_string();
        state.password = String::new();
        state.media_roots = vec![];

        let mut terminal = Terminal::new(TestBackend::new(70, 20)).unwrap();
        terminal.draw(|frame| {
            let wf = keybinding_bar::WindowFrame::new(frame.area());
            settings::render_settings(wf.content, frame.buffer_mut(), &state);
            wf.render_bar(frame.buffer_mut(), settings::keybindings());
        }).unwrap();
        assert_snapshot!(terminal.backend());
    }
}
