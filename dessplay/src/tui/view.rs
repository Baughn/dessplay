//! Pure `view()` function: `(UiState, DisplayData) → ViewSpec`.
//!
//! No I/O, no locks, no framework types.  Produces a complete declarative
//! description of the screen that a renderer can consume.

use std::sync::atomic::Ordering;

use dessplay_core::types::{FileState, UserState};
use dessplay_core::view_spec::*;

use crate::tui::display_data::{DisplayData, PlaylistDisplayEntry, UserDisplayEntry};
use crate::tui::ui_state::{
    ConnectingState, FocusedPane, MetadataAssignStep, Screen, UiState,
};

// =========================================================================
// Public entry point
// =========================================================================

/// Build a complete `ViewSpec` from UI state and pre-computed display data.
pub fn view(ui: &UiState, data: &DisplayData) -> ViewSpec {
    let base = main_layout(ui, data);
    let mut modals = Vec::new();

    // Modal stack (order matters: later = on top)
    if let Some(ref settings) = ui.settings
        && ui.screen == Screen::Settings
    {
        modals.push(settings_modal(settings));
    }
    if let Some(ref fb) = ui.file_browser
        && ui.screen == Screen::FileBrowser
    {
        modals.push(file_browser_modal(fb));
    }
    if let Some(ref tofu) = ui.tofu_warning
        && ui.screen == Screen::TofuWarning
    {
        modals.push(tofu_warning_modal(tofu));
    }
    if let Some(ref hashing) = ui.hashing
        && ui.screen == Screen::Hashing
    {
        modals.push(hashing_modal(hashing));
    }
    if let Some(ref ma) = ui.metadata_assign
        && ui.screen == Screen::MetadataAssign
    {
        modals.push(metadata_assign_modal(ma));
    }
    if let Some(ref connecting) = ui.connecting
        && ui.screen == Screen::Connecting
    {
        modals.push(connecting_modal(connecting));
    }

    // Status bar: collect visible bindings from topmost modal or focused pane
    let status_bar = build_status_bar(&modals, &base);

    ViewSpec {
        base,
        modals,
        status_bar: Some(status_bar),
    }
}

// =========================================================================
// Main layout
// =========================================================================

fn main_layout(ui: &UiState, data: &DisplayData) -> LayoutNode {
    let chat = chat_pane(ui, data);
    let chat_input = chat_input_pane(ui);
    let recent = recent_series_pane(ui, data);
    let users = users_pane(data);
    let playlist = playlist_pane(ui, data);
    let player = player_status_spacer(data);

    // Right column: recent series (30%) / users (30%) / playlist (40%)
    let right_top = LayoutNode::VSplit {
        top: Box::new(recent),
        bottom: Box::new(users),
        ratio: 0.5, // 50/50 of the top 60%
    };
    let right = LayoutNode::VSplit {
        top: Box::new(right_top),
        bottom: Box::new(playlist),
        ratio: 0.6, // top 60%, bottom 40%
    };

    // Main columns: chat 50% | right 50%
    let columns = LayoutNode::HSplit {
        left: Box::new(chat),
        right: Box::new(right),
        ratio: 0.5,
    };

    // Content area: columns above, chat input (1 line, full width) below
    let content = LayoutNode::VSplit {
        top: Box::new(columns),
        bottom: Box::new(chat_input),
        ratio: 1.0, // chat_input is fixed-height; handled by renderer
    };

    // Full screen: content above, player status (fixed 5 lines) below
    LayoutNode::VSplit {
        top: Box::new(content),
        bottom: Box::new(player),
        ratio: 1.0, // player_status is fixed-height; handled by renderer
    }
}

// =========================================================================
// Pane builders
// =========================================================================

fn chat_pane(ui: &UiState, data: &DisplayData) -> LayoutNode {
    let focused = ui.focus == FocusedPane::Chat;
    let lines: Vec<Vec<StyledSpan>> = data
        .chat_messages
        .iter()
        .map(|(uid, text)| {
            vec![
                StyledSpan::bold(format!("{}: ", uid.0), SemanticColor::Username),
                StyledSpan::plain(text),
            ]
        })
        .collect();

    LayoutNode::Pane(PaneSpec {
        id: PaneId::Chat,
        title: "Chat".to_string(),
        focused,
        content: ContentKind::TextLog {
            lines,
            scroll_back: ui.chat_scroll,
        },
        bindings: chat_bindings(),
    })
}

fn chat_input_pane(ui: &UiState) -> LayoutNode {
    let focused = ui.focus == FocusedPane::Chat;
    LayoutNode::Pane(PaneSpec {
        id: PaneId::ChatInput,
        title: String::new(),
        focused,
        content: ContentKind::TextInput {
            text: ui.input.text.clone(),
            cursor_pos: ui.input.cursor,
            placeholder: String::new(),
        },
        bindings: Vec::new(), // input bindings are on the Chat pane
    })
}

fn users_pane(data: &DisplayData) -> LayoutNode {
    let items: Vec<Vec<StyledSpan>> = data
        .user_entries
        .iter()
        .map(user_entry_spans)
        .collect();

    LayoutNode::Pane(PaneSpec {
        id: PaneId::Users,
        title: "Users".to_string(),
        focused: false, // users pane is never focused
        content: ContentKind::SelectableList {
            items,
            selected: 0,
            scroll_offset: 0,
            highlighted: None,
        },
        bindings: Vec::new(),
    })
}

fn playlist_pane(ui: &UiState, data: &DisplayData) -> LayoutNode {
    let focused = ui.focus == FocusedPane::Playlist;
    let mut items: Vec<Vec<StyledSpan>> = data
        .playlist_entries
        .iter()
        .map(playlist_entry_spans)
        .collect();
    items.push(vec![StyledSpan::colored("[Add New]", SemanticColor::Muted)]);

    let highlighted = data
        .playlist_entries
        .iter()
        .position(|e| e.is_current);

    LayoutNode::Pane(PaneSpec {
        id: PaneId::Playlist,
        title: "Playlist".to_string(),
        focused,
        content: ContentKind::SelectableList {
            items,
            selected: ui.playlist_selected,
            scroll_offset: 0,
            highlighted,
        },
        bindings: playlist_bindings(),
    })
}

fn recent_series_pane(ui: &UiState, data: &DisplayData) -> LayoutNode {
    let focused = ui.focus == FocusedPane::RecentSeries;
    let items: Vec<Vec<StyledSpan>> = data
        .series_entries
        .iter()
        .map(|entry| {
            let color = if entry.has_unwatched {
                SemanticColor::Ready
            } else {
                SemanticColor::Muted
            };
            vec![StyledSpan::colored(&entry.name, color)]
        })
        .collect();

    LayoutNode::Pane(PaneSpec {
        id: PaneId::RecentSeries,
        title: "Recent Series".to_string(),
        focused,
        content: ContentKind::SelectableList {
            items,
            selected: ui.recent_selected,
            scroll_offset: 0,
            highlighted: None,
        },
        bindings: recent_series_bindings(),
    })
}

fn player_status_spacer(data: &DisplayData) -> LayoutNode {
    let mut children = Vec::new();

    // Line 1: progress bar
    let fraction = match data.duration_secs {
        Some(dur) if dur > 0.0 => (data.position_secs / dur).clamp(0.0, 1.0),
        _ => 0.0,
    };
    let play_indicator = if data.is_playing { ">" } else { "||" };
    let time_label = format!(
        "{} {} / {}",
        play_indicator,
        format_time(data.position_secs),
        data.duration_secs
            .map(format_time)
            .unwrap_or_else(|| "--:--".to_string()),
    );
    children.push(ContentKind::ProgressBar {
        fraction,
        label: time_label,
    });

    // Line 2: now playing + optional bg hash
    let mut line2 = Vec::new();
    if let Some(ref name) = data.current_file_name {
        line2.push(StyledSpan::plain(format!("Now Playing: {name}")));
    }
    if let Some(ref bg) = data.bg_hash_progress {
        let mut parts = format!("  [Indexing: {}/{}", bg.completed_files, bg.total_files);
        if let Some(ref name) = bg.current_file {
            parts.push(' ');
            parts.push_str(name);
        }
        if let Some(rate) = bg.rate_bps {
            parts.push_str(&format!(" | {}/s", format_bytes(rate as u64)));
        }
        if let Some(eta) = bg.eta_secs
            && eta > 0.0
        {
            parts.push_str(&format!(" | ETA {}", format_eta(eta)));
        }
        parts.push(']');
        line2.push(StyledSpan::colored(parts, SemanticColor::Muted));
    }
    if !line2.is_empty() {
        children.push(ContentKind::TextLog {
            lines: vec![line2],
            scroll_back: 0,
        });
    }

    // Line 3: blocking users
    if !data.blocking_users.is_empty() {
        let waiting = format!("Waiting for: {}", data.blocking_users.join(", "));
        children.push(ContentKind::TextLog {
            lines: vec![vec![StyledSpan::colored(waiting, SemanticColor::Paused)]],
            scroll_back: 0,
        });
    }

    LayoutNode::Spacer {
        height: 5,
        content: ContentKind::Composite { children },
    }
}

// =========================================================================
// Modal builders
// =========================================================================

fn settings_modal(
    settings: &crate::tui::ui_state::SettingsState,
) -> ModalSpec {
    let mut fields = Vec::new();

    // Username
    fields.push(FormField {
        label: "Username".to_string(),
        kind: FormFieldKind::Text {
            value: settings.username.clone(),
        },
        error: if !settings.is_username_valid() {
            Some("required".to_string())
        } else {
            None
        },
    });

    // Server
    fields.push(FormField {
        label: "Server".to_string(),
        kind: FormFieldKind::Text {
            value: settings.server.clone(),
        },
        error: settings.server_error().map(|s| s.to_string()),
    });

    // Player
    fields.push(FormField {
        label: "Player".to_string(),
        kind: FormFieldKind::Toggle {
            value: settings.player.clone(),
            choices: vec!["mpv".to_string(), "vlc".to_string()],
        },
        error: None,
    });

    // Password
    fields.push(FormField {
        label: "Password".to_string(),
        kind: FormFieldKind::Masked {
            value: settings.password.clone(),
        },
        error: if !settings.is_password_valid() {
            Some("required (or set DESSPLAY_PASSWORD)".to_string())
        } else {
            None
        },
    });

    // Media roots — prepend [Add New] pseudo-entry
    let mut paths = vec![PathListEntry {
        path: "[Add New]".into(),
        is_download_target: false,
    }];
    paths.extend(settings.media_roots.iter().enumerate().map(|(i, p)| {
        PathListEntry {
            path: p.display().to_string(),
            is_download_target: i == 0,
        }
    }));
    fields.push(FormField {
        label: "Media Roots".to_string(),
        kind: FormFieldKind::PathList {
            paths,
            selected: settings.media_root_selected,
        },
        error: if !settings.has_media_roots() {
            Some("at least one required".to_string())
        } else {
            None
        },
    });

    // Validation hint
    let hint = if settings.is_valid() {
        Some(vec![StyledSpan::colored(
            "Ctrl-S to save",
            SemanticColor::Ready,
        )])
    } else {
        let mut parts = Vec::new();
        if !settings.is_username_valid() {
            parts.push("username required");
        }
        if !settings.is_server_valid() {
            parts.push("invalid server");
        }
        if !settings.is_password_valid() {
            parts.push("password required");
        }
        if !settings.has_media_roots() {
            parts.push("add a media root");
        }
        Some(vec![StyledSpan::colored(
            parts.join(", "),
            SemanticColor::Error,
        )])
    };

    ModalSpec {
        title: " Settings ".to_string(),
        width_pct: 0.9,
        height_pct: 0.8,
        content: ContentKind::Form {
            fields,
            focused_field: settings.focused_field,
            alert: settings.alert.clone(),
            hint,
        },
        bindings: settings_bindings(settings.focused_field),
    }
}

fn file_browser_modal(
    fb: &crate::tui::ui_state::FileBrowserState,
) -> ModalSpec {
    let title = format!(
        " {} ",
        fb.current_dir.display()
    );

    let items: Vec<Vec<StyledSpan>> = fb
        .entries
        .iter()
        .map(|entry| {
            let color = if entry.is_dir {
                SemanticColor::Accent
            } else {
                SemanticColor::Default
            };
            let suffix = if entry.is_dir { "/" } else { "" };
            vec![StyledSpan::colored(
                format!("{}{suffix}", entry.name),
                color,
            )]
        })
        .collect();

    ModalSpec {
        title,
        width_pct: 0.8,
        height_pct: 0.7,
        content: ContentKind::SelectableList {
            items,
            selected: fb.selected,
            scroll_offset: fb.scroll_offset,
            highlighted: None,
        },
        bindings: file_browser_bindings(fb),
    }
}

fn tofu_warning_modal(
    tofu: &crate::tui::ui_state::TofuWarningState,
) -> ModalSpec {
    let stored_fp = tofu
        .stored_fingerprint
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":");
    let received_fp = tofu
        .received_fingerprint
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":");

    let lines = vec![
        vec![StyledSpan::colored(
            "WARNING: Server certificate has changed!",
            SemanticColor::Error,
        )],
        vec![StyledSpan::plain(format!("Server: {}", tofu.server))],
        vec![],
        vec![StyledSpan::plain(format!("Stored:   {stored_fp}"))],
        vec![StyledSpan::plain(format!("Received: {received_fp}"))],
        vec![],
        vec![StyledSpan::plain(
            "Ctrl-F to accept, Esc to reject",
        )],
    ];

    ModalSpec {
        title: " Certificate Mismatch ".to_string(),
        width_pct: 0.8,
        height_pct: 0.4,
        content: ContentKind::TextLog {
            lines,
            scroll_back: 0,
        },
        bindings: tofu_bindings(),
    }
}

fn hashing_modal(hashing: &crate::tui::ui_state::HashingState) -> ModalSpec {
    let bytes_done = hashing.bytes_hashed.load(Ordering::Relaxed);
    let total = hashing.total_bytes;
    let fraction = if total > 0 {
        (bytes_done as f64 / total as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let percent = (fraction * 100.0) as u64;

    let done_str = format_bytes(bytes_done);
    let total_str = format_bytes(total);

    let lines = vec![
        vec![StyledSpan::bold(&hashing.filename, SemanticColor::Default)],
        vec![StyledSpan::colored(
            format!("{percent}% — {done_str} / {total_str}"),
            SemanticColor::Accent,
        )],
    ];

    ModalSpec {
        title: " Hashing file ".to_string(),
        width_pct: 0.5,
        height_pct: 0.3,
        content: ContentKind::Composite {
            children: vec![
                ContentKind::TextLog {
                    lines,
                    scroll_back: 0,
                },
                ContentKind::ProgressBar {
                    fraction,
                    label: format!("{percent}%"),
                },
            ],
        },
        bindings: Vec::new(), // only Ctrl-C works during hashing
    }
}

fn metadata_assign_modal(
    ma: &crate::tui::ui_state::MetadataAssignState,
) -> ModalSpec {
    match ma.step {
        MetadataAssignStep::SelectSeries => {
            let items: Vec<Vec<StyledSpan>> = ma
                .series_list
                .iter()
                .map(|s| vec![StyledSpan::plain(&s.name)])
                .collect();

            ModalSpec {
                title: " Select Series ".to_string(),
                width_pct: 0.5,
                height_pct: 0.5,
                content: ContentKind::SelectableList {
                    items,
                    selected: ma.selected,
                    scroll_offset: 0,
                    highlighted: None,
                },
                bindings: metadata_select_bindings(),
            }
        }
        MetadataAssignStep::EnterEpisode => {
            let series_name = ma
                .series_list
                .get(ma.selected)
                .map(|s| s.name.as_str())
                .unwrap_or("?");

            let lines = vec![vec![StyledSpan::plain(format!(
                "Series: {series_name}"
            ))]];

            ModalSpec {
                title: " Enter Episode Number ".to_string(),
                width_pct: 0.5,
                height_pct: 0.3,
                content: ContentKind::Composite {
                    children: vec![
                        ContentKind::TextLog {
                            lines,
                            scroll_back: 0,
                        },
                        ContentKind::TextInput {
                            text: ma.episode_input.text.clone(),
                            cursor_pos: ma.episode_input.cursor,
                            placeholder: "e.g. 01, S1, C1".to_string(),
                        },
                    ],
                },
                bindings: metadata_episode_bindings(),
            }
        }
    }
}

fn connecting_modal(state: &ConnectingState) -> ModalSpec {
    let lines = vec![
        vec![],
        vec![StyledSpan::bold(
            format!("Connecting to {}...", state.server),
            SemanticColor::Accent,
        )],
    ];

    ModalSpec {
        title: " Connecting ".to_string(),
        width_pct: 0.5,
        height_pct: 0.3,
        content: ContentKind::TextLog {
            lines,
            scroll_back: 0,
        },
        bindings: connecting_bindings(),
    }
}

// =========================================================================
// Keybinding declarations
// =========================================================================

fn chat_bindings() -> Vec<Keybinding> {
    vec![
        kb_bar(KeyCombo::Plain(Key::Tab), "Next pane", Action::CycleFocus),
        kb_bar(KeyCombo::Plain(Key::Enter), "Send", Action::SendChat),
        kb_bar(KeyCombo::Plain(Key::Esc), "Clear", Action::ClearInput),
        kb_bar(KeyCombo::Ctrl(Key::Char('s')), "Settings", Action::OpenSettings),
        kb_bar(KeyCombo::Ctrl(Key::Char('c')), "Quit", Action::Quit),
        // Hidden bindings (not shown in bar)
        kb(KeyCombo::Plain(Key::Backspace), "Delete", Action::DeleteBack),
        kb(KeyCombo::Plain(Key::Delete), "Delete fwd", Action::DeleteForward),
        kb(KeyCombo::Plain(Key::Left), "Left", Action::CursorLeft),
        kb(KeyCombo::Plain(Key::Right), "Right", Action::CursorRight),
        kb(KeyCombo::Ctrl(Key::Left), "Word left", Action::CursorWordLeft),
        kb(KeyCombo::Ctrl(Key::Right), "Word right", Action::CursorWordRight),
        kb(KeyCombo::Plain(Key::Home), "Home", Action::CursorHome),
        kb(KeyCombo::Plain(Key::End), "End", Action::CursorEnd),
        kb(KeyCombo::Ctrl(Key::Char('w')), "Del word", Action::DeleteWordBack),
        kb(KeyCombo::Plain(Key::Up), "Scroll up", Action::ScrollChatUp),
        kb(KeyCombo::Plain(Key::Down), "Scroll down", Action::ScrollChatDown),
        kb(KeyCombo::Plain(Key::PageUp), "Page up", Action::ScrollChatPageUp),
        kb(KeyCombo::Plain(Key::PageDown), "Page dn", Action::ScrollChatPageDown),
        kb(KeyCombo::Ctrl(Key::Up), "Page up", Action::ScrollChatPageUp),
        kb(KeyCombo::Ctrl(Key::Down), "Page dn", Action::ScrollChatPageDown),
        // Char input is handled specially by resolve_input, not listed here
    ]
}

fn playlist_bindings() -> Vec<Keybinding> {
    vec![
        kb_bar(KeyCombo::Plain(Key::Tab), "Next pane", Action::CycleFocus),
        kb_bar(KeyCombo::Plain(Key::Enter), "Play", Action::SetNowPlaying),
        kb_bar(KeyCombo::Plain(Key::Char('a')), "Add", Action::OpenFileBrowser),
        kb_bar(KeyCombo::Plain(Key::Char('d')), "Remove", Action::PlaylistRemove),
        kb_bar(KeyCombo::Ctrl(Key::Char('j')), "Move dn", Action::PlaylistMoveDown),
        kb_bar(KeyCombo::Ctrl(Key::Char('k')), "Move up", Action::PlaylistMoveUp),
        kb_bar(KeyCombo::Ctrl(Key::Char('s')), "Settings", Action::OpenSettings),
        kb_bar(KeyCombo::Ctrl(Key::Char('c')), "Quit", Action::Quit),
        kb(KeyCombo::Plain(Key::Up), "Up", Action::PlaylistSelectUp),
        kb(KeyCombo::Plain(Key::Down), "Down", Action::PlaylistSelectDown),
        kb(KeyCombo::Plain(Key::PageUp), "Page up", Action::PlaylistPageUp),
        kb(KeyCombo::Plain(Key::PageDown), "Page dn", Action::PlaylistPageDown),
        kb(KeyCombo::Ctrl(Key::Up), "Page up", Action::PlaylistPageUp),
        kb(KeyCombo::Ctrl(Key::Down), "Page dn", Action::PlaylistPageDown),
        kb(KeyCombo::Ctrl(Key::Char('a')), "Assign meta", Action::AssignMetadata),
        kb(KeyCombo::Ctrl(Key::Char('m')), "Map file", Action::ManualMapFile),
    ]
}

fn recent_series_bindings() -> Vec<Keybinding> {
    vec![
        kb_bar(KeyCombo::Plain(Key::Tab), "Next pane", Action::CycleFocus),
        kb_bar(KeyCombo::Plain(Key::Enter), "Browse", Action::RecentSeriesSelect),
        kb_bar(KeyCombo::Ctrl(Key::Char('s')), "Settings", Action::OpenSettings),
        kb_bar(KeyCombo::Ctrl(Key::Char('c')), "Quit", Action::Quit),
        kb(KeyCombo::Plain(Key::Up), "Up", Action::RecentSelectUp),
        kb(KeyCombo::Plain(Key::Down), "Down", Action::RecentSelectDown),
        kb(KeyCombo::Plain(Key::PageUp), "Page up", Action::RecentPageUp),
        kb(KeyCombo::Plain(Key::PageDown), "Page dn", Action::RecentPageDown),
        kb(KeyCombo::Ctrl(Key::Up), "Page up", Action::RecentPageUp),
        kb(KeyCombo::Ctrl(Key::Down), "Page dn", Action::RecentPageDown),
    ]
}

fn settings_bindings(focused_field: usize) -> Vec<Keybinding> {
    let mut bindings = vec![
        kb_bar(KeyCombo::Plain(Key::Tab), "Next field", Action::SettingsNextField),
        kb_bar(KeyCombo::Shift(Key::Tab), "Prev field", Action::SettingsPrevField),
        kb_bar(KeyCombo::Ctrl(Key::Char('s')), "Save", Action::SettingsSave),
        kb_bar(KeyCombo::Plain(Key::Esc), "Cancel", Action::SettingsCancel),
        kb_bar(KeyCombo::Ctrl(Key::Char('c')), "Quit", Action::Quit),
    ];

    // Up/Down: navigate within media root list when focused, otherwise move between fields
    if focused_field == 4 {
        bindings.push(kb(KeyCombo::Plain(Key::Up), "Up", Action::SettingsMediaRootUp));
        bindings.push(kb(KeyCombo::Plain(Key::Down), "Down", Action::SettingsMediaRootDown));
        bindings.push(kb(KeyCombo::Plain(Key::PageUp), "Page up", Action::SettingsMediaRootPageUp));
        bindings.push(kb(KeyCombo::Plain(Key::PageDown), "Page dn", Action::SettingsMediaRootPageDown));
        bindings.push(kb(KeyCombo::Ctrl(Key::Up), "Page up", Action::SettingsMediaRootPageUp));
        bindings.push(kb(KeyCombo::Ctrl(Key::Down), "Page dn", Action::SettingsMediaRootPageDown));
    } else {
        bindings.push(kb(KeyCombo::Plain(Key::Up), "Prev field", Action::SettingsPrevField));
        bindings.push(kb(KeyCombo::Plain(Key::Down), "Next field", Action::SettingsNextField));
    }

    // Backspace and Ctrl-W only on text fields (username=0, server=1, password=3)
    match focused_field {
        0 | 1 | 3 => {
            bindings.push(kb(KeyCombo::Plain(Key::Backspace), "Delete", Action::SettingsDeleteBack));
            bindings.push(kb(KeyCombo::Ctrl(Key::Char('w')), "Del word", Action::SettingsDeleteWordBack));
        }
        _ => {}
    }

    // Context-dependent Enter and field-specific bindings
    match focused_field {
        2 => bindings.push(kb(KeyCombo::Plain(Key::Enter), "Toggle", Action::SettingsTogglePlayer)),
        4 => {
            bindings.push(kb_bar(KeyCombo::Plain(Key::Enter), "Select", Action::SettingsAddMediaRoot));
            bindings.push(kb(KeyCombo::Plain(Key::Char('d')), "Remove", Action::SettingsRemoveMediaRoot));
            bindings.push(kb(KeyCombo::Ctrl(Key::Char('j')), "Move dn", Action::SettingsMoveRootDown));
            bindings.push(kb(KeyCombo::Ctrl(Key::Char('k')), "Move up", Action::SettingsMoveRootUp));
        }
        _ => {}
    }

    // Char input is synthesized by resolve_input for Form content (text/masked fields)
    bindings
}

fn file_browser_bindings(
    fb: &crate::tui::ui_state::FileBrowserState,
) -> Vec<Keybinding> {
    let mut bindings = vec![
        kb_bar(KeyCombo::Plain(Key::Enter), "Select", Action::FileBrowserSelect),
        kb_bar(KeyCombo::Plain(Key::Esc), "Back", Action::FileBrowserBack),
        kb_bar(KeyCombo::Ctrl(Key::Char('c')), "Quit", Action::Quit),
        kb(KeyCombo::Plain(Key::Up), "Up", Action::FileBrowserUp),
        kb(KeyCombo::Plain(Key::Down), "Down", Action::FileBrowserDown),
        kb(KeyCombo::Plain(Key::PageUp), "Page up", Action::FileBrowserPageUp),
        kb(KeyCombo::Plain(Key::PageDown), "Page dn", Action::FileBrowserPageDown),
        kb(KeyCombo::Ctrl(Key::Up), "Page up", Action::FileBrowserPageUp),
        kb(KeyCombo::Ctrl(Key::Down), "Page dn", Action::FileBrowserPageDown),
    ];
    if matches!(
        fb.origin,
        crate::tui::ui_state::FileBrowserOrigin::SettingsMediaRoot
    ) {
        bindings.push(kb_bar(
            KeyCombo::Plain(Key::Char('s')),
            "Select dir",
            Action::FileBrowserSelectDir,
        ));
    }
    bindings
}

fn tofu_bindings() -> Vec<Keybinding> {
    vec![
        kb_bar(KeyCombo::Ctrl(Key::Char('f')), "Accept", Action::TofuAccept),
        kb_bar(KeyCombo::Plain(Key::Esc), "Reject", Action::TofuReject),
        kb_bar(KeyCombo::Ctrl(Key::Char('c')), "Quit", Action::Quit),
    ]
}

fn metadata_select_bindings() -> Vec<Keybinding> {
    vec![
        kb_bar(KeyCombo::Plain(Key::Enter), "Select", Action::MetadataConfirmSeries),
        kb_bar(KeyCombo::Plain(Key::Esc), "Cancel", Action::MetadataCancel),
        kb_bar(KeyCombo::Ctrl(Key::Char('c')), "Quit", Action::Quit),
        kb(KeyCombo::Plain(Key::Up), "Up", Action::MetadataSelectUp),
        kb(KeyCombo::Plain(Key::Down), "Down", Action::MetadataSelectDown),
        kb(KeyCombo::Plain(Key::PageUp), "Page up", Action::MetadataPageUp),
        kb(KeyCombo::Plain(Key::PageDown), "Page dn", Action::MetadataPageDown),
        kb(KeyCombo::Ctrl(Key::Up), "Page up", Action::MetadataPageUp),
        kb(KeyCombo::Ctrl(Key::Down), "Page dn", Action::MetadataPageDown),
    ]
}

fn connecting_bindings() -> Vec<Keybinding> {
    vec![
        kb_bar(KeyCombo::Plain(Key::Esc), "Back", Action::CancelConnect),
        kb_bar(KeyCombo::Ctrl(Key::Char('c')), "Quit", Action::Quit),
    ]
}

fn metadata_episode_bindings() -> Vec<Keybinding> {
    vec![
        kb_bar(KeyCombo::Plain(Key::Enter), "Confirm", Action::MetadataConfirmEpisode),
        kb_bar(KeyCombo::Plain(Key::Esc), "Cancel", Action::MetadataCancel),
        kb_bar(KeyCombo::Ctrl(Key::Char('c')), "Quit", Action::Quit),
        kb(KeyCombo::Plain(Key::Backspace), "Delete", Action::MetadataDeleteBack),
        // Char input handled specially by resolve_input
    ]
}

// =========================================================================
// Keybinding helpers
// =========================================================================

fn kb(key: KeyCombo, label: &'static str, action: Action) -> Keybinding {
    Keybinding {
        key,
        label,
        action,
        show_in_bar: false,
    }
}

fn kb_bar(key: KeyCombo, label: &'static str, action: Action) -> Keybinding {
    Keybinding {
        key,
        label,
        action,
        show_in_bar: true,
    }
}

// =========================================================================
// Status bar
// =========================================================================

fn build_status_bar(modals: &[ModalSpec], base: &LayoutNode) -> StatusBarSpec {
    // Collect bindings from the topmost modal, or the focused pane
    let bindings_source = if let Some(modal) = modals.last() {
        &modal.bindings
    } else {
        find_focused_pane_bindings(base)
            .unwrap_or(&[])
    };

    let bindings: Vec<(String, &'static str)> = bindings_source
        .iter()
        .filter(|b| b.show_in_bar)
        .map(|b| (format_key_combo(&b.key), b.label))
        .collect();

    StatusBarSpec { bindings }
}

fn find_focused_pane_bindings(node: &LayoutNode) -> Option<&[Keybinding]> {
    match node {
        LayoutNode::Pane(pane) if pane.focused => Some(&pane.bindings),
        LayoutNode::HSplit { left, right, .. } => {
            find_focused_pane_bindings(left).or_else(|| find_focused_pane_bindings(right))
        }
        LayoutNode::VSplit { top, bottom, .. } => {
            find_focused_pane_bindings(top).or_else(|| find_focused_pane_bindings(bottom))
        }
        _ => None,
    }
}

fn format_key_combo(key: &KeyCombo) -> String {
    match key {
        KeyCombo::Plain(k) => format_key(k),
        KeyCombo::Ctrl(k) => format!("Ctrl-{}", format_key(k)),
        KeyCombo::Shift(k) => format!("Shift-{}", format_key(k)),
    }
}

fn format_key(key: &Key) -> String {
    match key {
        Key::Char(c) => c.to_string(),
        Key::Enter => "Enter".to_string(),
        Key::Esc => "Esc".to_string(),
        Key::Tab => "Tab".to_string(),
        Key::Backspace => "Bksp".to_string(),
        Key::Delete => "Del".to_string(),
        Key::Up => "Up".to_string(),
        Key::Down => "Down".to_string(),
        Key::Left => "Left".to_string(),
        Key::Right => "Right".to_string(),
        Key::Home => "Home".to_string(),
        Key::End => "End".to_string(),
        Key::PageUp => "PgUp".to_string(),
        Key::PageDown => "PgDn".to_string(),
    }
}

// =========================================================================
// Display helpers
// =========================================================================

fn user_entry_spans(entry: &UserDisplayEntry) -> Vec<StyledSpan> {
    let (color, status_text) = user_display_color(entry);
    let suffix = if entry.is_self { " (you)" } else { "" };
    vec![
        StyledSpan::colored(format!("{}{suffix}", entry.name), color),
        StyledSpan::colored(format!(" [{status_text}]"), color),
    ]
}

fn user_display_color(entry: &UserDisplayEntry) -> (SemanticColor, &'static str) {
    match (&entry.user_state, &entry.file_state) {
        (UserState::Paused, _) => (SemanticColor::Paused, "Paused"),
        (UserState::NotWatching, _) => (SemanticColor::NotWatching, "Not watching"),
        (UserState::Ready, FileState::Missing) => (SemanticColor::Missing, "Missing"),
        (UserState::Ready, FileState::Downloading { progress }) if *progress >= 0.2 => {
            (SemanticColor::Ready, "Downloading")
        }
        (_, FileState::Downloading { .. }) => (SemanticColor::Downloading, "Downloading"),
        (UserState::Ready, FileState::Ready) => (SemanticColor::Ready, "Ready"),
    }
}

fn playlist_entry_spans(entry: &PlaylistDisplayEntry) -> Vec<StyledSpan> {
    let color = if entry.is_missing {
        SemanticColor::Missing
    } else if entry.is_played {
        SemanticColor::Muted
    } else {
        SemanticColor::Default
    };
    vec![StyledSpan::colored(&entry.display_name, color)]
}

fn format_time(secs: f64) -> String {
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.0} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn format_eta(secs: f64) -> String {
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else if m > 0 {
        format!("{m}:{s:02}")
    } else {
        format!("{s}s")
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::tui::display_data::DisplayData;
    use crate::tui::ui_state::UiState;
    use dessplay_core::types::{FileId, FileState, UserState};

    fn fid(n: u8) -> FileId {
        let mut id = [0u8; 16];
        id[0] = n;
        FileId(id)
    }

    fn empty_display_data() -> DisplayData {
        DisplayData {
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

    #[test]
    fn view_produces_base_layout() {
        let ui = UiState::new();
        let data = empty_display_data();
        let spec = view(&ui, &data);
        // Base should be a VSplit (main content | player status)
        assert!(matches!(spec.base, LayoutNode::VSplit { .. }));
        assert!(spec.modals.is_empty());
        assert!(spec.status_bar.is_some());
    }

    #[test]
    fn view_chat_focus_bindings() {
        let ui = UiState::new(); // default focus = Chat
        let data = empty_display_data();
        let spec = view(&ui, &data);
        let bar = spec.status_bar.unwrap();
        let labels: Vec<&str> = bar.bindings.iter().map(|(_, l)| *l).collect();
        assert!(labels.contains(&"Send"), "Chat bar should have Send");
        assert!(labels.contains(&"Next pane"), "Chat bar should have Tab");
    }

    #[test]
    fn view_playlist_focus_bindings() {
        let mut ui = UiState::new();
        ui.focus = FocusedPane::Playlist;
        let data = empty_display_data();
        let spec = view(&ui, &data);
        let bar = spec.status_bar.unwrap();
        let labels: Vec<&str> = bar.bindings.iter().map(|(_, l)| *l).collect();
        assert!(labels.contains(&"Add"), "Playlist bar should have Add");
        assert!(labels.contains(&"Remove"), "Playlist bar should have Remove");
    }

    #[test]
    fn view_settings_modal_appears() {
        let mut ui = UiState::new();
        ui.screen = Screen::Settings;
        ui.settings = Some(crate::tui::ui_state::SettingsState::new());
        let data = empty_display_data();
        let spec = view(&ui, &data);
        assert_eq!(spec.modals.len(), 1);
        assert!(spec.modals[0].title.contains("Settings"));
    }

    #[test]
    fn view_connecting_modal_appears() {
        let mut ui = UiState::new();
        ui.screen = Screen::Connecting;
        ui.connecting = Some(crate::tui::ui_state::ConnectingState {
            server: "dessplay.brage.info:4433".to_string(),
        });
        let data = empty_display_data();
        let spec = view(&ui, &data);
        assert_eq!(spec.modals.len(), 1);
        assert!(spec.modals[0].title.contains("Connecting"));
        // Status bar should show Back and Quit
        let bar = spec.status_bar.unwrap();
        let labels: Vec<&str> = bar.bindings.iter().map(|(_, l)| *l).collect();
        assert!(labels.contains(&"Back"), "Connecting bar should have Back");
        assert!(labels.contains(&"Quit"), "Connecting bar should have Quit");
    }

    #[test]
    fn view_user_entries_colored() {
        let data = DisplayData {
            user_entries: vec![
                UserDisplayEntry {
                    name: "alice".to_string(),
                    user_state: UserState::Ready,
                    file_state: FileState::Ready,
                    is_self: true,
                },
                UserDisplayEntry {
                    name: "bob".to_string(),
                    user_state: UserState::Paused,
                    file_state: FileState::Ready,
                    is_self: false,
                },
            ],
            ..empty_display_data()
        };
        let spans = user_entry_spans(&data.user_entries[0]);
        assert!(spans[0].text.contains("(you)"));
        assert_eq!(spans[1].color, SemanticColor::Ready);

        let spans = user_entry_spans(&data.user_entries[1]);
        assert_eq!(spans[1].color, SemanticColor::Paused);
    }

    #[test]
    fn view_playlist_missing_colored_red() {
        let entry = PlaylistDisplayEntry {
            file_id: fid(1),
            display_name: "test.mkv".to_string(),
            is_missing: true,
            is_current: true,
            is_played: false,
        };
        let spans = playlist_entry_spans(&entry);
        assert_eq!(spans[0].color, SemanticColor::Missing);
    }

    #[test]
    fn view_metadata_modal_appears() {
        let mut ui = UiState::new();
        ui.screen = Screen::MetadataAssign;
        ui.metadata_assign = Some(crate::tui::ui_state::MetadataAssignState {
            file_id: fid(1),
            series_list: vec![crate::tui::ui_state::SeriesChoice {
                anime_id: 42,
                name: "Frieren".to_string(),
            }],
            selected: 0,
            step: MetadataAssignStep::SelectSeries,
            episode_input: crate::tui::ui_state::InputState::new(),
        });
        let data = empty_display_data();
        let spec = view(&ui, &data);
        assert_eq!(spec.modals.len(), 1);
        assert!(spec.modals[0].title.contains("Series"));
    }

    #[test]
    fn format_time_short() {
        assert_eq!(format_time(0.0), "0:00");
        assert_eq!(format_time(65.0), "1:05");
        assert_eq!(format_time(3661.0), "1:01:01");
    }

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1 KiB");
        assert_eq!(format_bytes(1024 * 1024 * 5), "5.0 MiB");
    }

    #[test]
    fn format_eta_seconds() {
        assert_eq!(format_eta(45.0), "45s");
        assert_eq!(format_eta(0.0), "0s");
        assert_eq!(format_eta(59.0), "59s");
    }

    #[test]
    fn format_eta_minutes() {
        assert_eq!(format_eta(60.0), "1:00");
        assert_eq!(format_eta(272.0), "4:32");
        assert_eq!(format_eta(3599.0), "59:59");
    }

    #[test]
    fn format_eta_hours() {
        assert_eq!(format_eta(3600.0), "1:00:00");
        assert_eq!(format_eta(3930.0), "1:05:30");
    }

    #[test]
    fn view_modal_overrides_status_bar() {
        let mut ui = UiState::new();
        ui.screen = Screen::Settings;
        ui.settings = Some(crate::tui::ui_state::SettingsState::new());
        let data = empty_display_data();
        let spec = view(&ui, &data);
        let bar = spec.status_bar.unwrap();
        let labels: Vec<&str> = bar.bindings.iter().map(|(_, l)| *l).collect();
        // Settings modal should override the bar
        assert!(labels.contains(&"Save"), "Settings bar should have Save");
        assert!(!labels.contains(&"Send"), "Settings bar should not have chat Send");
    }
}
