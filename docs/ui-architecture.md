# UI Architecture

Last updated: 2026-06-12

DessPlay uses **tui-realm** as its TUI framework, providing an Elm-style
architecture on top of ratatui. This document covers the component structure,
message flow, and how the UI integrates with the actor system.

## Table of Contents

1. [Framework Choice](#framework-choice)
2. [Component Structure](#component-structure)
3. [Message Flow](#message-flow)
4. [Integration with Actor System](#integration-with-actor-system)
5. [Keybinding Bar](#keybinding-bar)
6. [Modals](#modals)
7. [Testing](#testing)
8. [Web Renderer (Future)](#web-renderer-future)

---

## Framework Choice

**tui-realm** provides:
- **Elm architecture**: Components have Props (input data), State (internal),
  and produce Msg (output). Unidirectional data flow.
- **Pre-built components**: `tui-realm-stdlib` (the chat/field `Input`).
- **ratatui ecosystem**: Can use any ratatui widget inside a tui-realm
  component.
- **Test helpers**: `tuirealm::testing` renders components to strings for
  insta snapshots.

**Deviation (Phase 6):** we use tui-realm's *component model*
(`Component`/`AppComponent`, `Event`, `Cmd`) but **not its `Application`
event loop**. tui-realm's listener routes input through real worker
threads polling on real-time intervals, which would make whole-app tests
timing-dependent — and deterministic, zero-thread UI tests are a core
testing-strategy commitment. Instead, `ui::app::Ui` is a ~100-line
synchronous dispatcher owning the focus ring, the modal stack,
event-to-component routing, and `update()`. Tests inject `Event`s and
read back `UserAction`s and rendered buffers with no threads anywhere;
production wraps the same `Ui` in two plain threads (`ui::shell`).

This still replaces the prototype's 2200-line `runner.rs` — with less
framework, not more hand-rolling.

---

## Component Structure

### Component Tree

```
Application
+-- ChatPane (TextLog + TextInput composite)
|   +-- ChatLog (scrollable message list)
|   +-- ChatInput (text input with cursor)
+-- SubtitlePane (rolling sub-text log; optional, splits ChatPane area)
+-- SeriesPane (SelectableList, three modes: Recent / All / The List)
+-- UsersPane (styled list; focusable for the Away action)
+-- PlaylistPane (SelectableList with actions)
+-- PlayerStatus (progress bar + info)
+-- KeybindingBar (derived from active focus)
```

### Component Definition Pattern

Each component follows tui-realm's `Component` trait:

```rust
impl Component<Msg, UserEvent> for PlaylistPane {
    fn on(&mut self, event: Event<UserEvent>) -> Option<Msg> {
        match event {
            // Keyboard events -> produce messages
            Event::Keyboard(KeyEvent { code: KeyCode::Enter, .. }) => {
                Some(Msg::PlaySelected(self.selected_index()))
            }
            Event::Keyboard(KeyEvent { code: KeyCode::Char('a'), .. }) => {
                Some(Msg::OpenFileBrowser)
            }
            // ...
            _ => None,
        }
    }

    fn view(&mut self, frame: &mut Frame, area: Rect) {
        // Render using ratatui widgets
    }
}
```

### Message Enum

```rust
enum Msg {
    // Chat
    SendChat(String),
    ChatInputChanged(String),

    // Series
    CycleSeriesMode,            // Recent -> All -> The List
    ToggleSeriesSort,
    BrowseFranchise(FranchiseId),

    // The List
    JumpToNextEp(ListEntryId),  // Enter on a linked Active entry
    EditListEntry(ListEntryId),
    LinkListEntry(ListEntryId), // open AniDB search modal

    // Users
    ToggleAway(UserId),

    // Playlist
    PlaySelected(usize),
    AddFile,
    MoveUp,
    MoveDown,
    RemoveSelected,
    ArchiveSelected,
    MapSelected,                // manual file mapping (Ctrl-m)

    // Player
    SeekTo(f64),

    // Navigation
    FocusNext,
    ToggleSubtitlePane,
    Quit,

    // Modals
    OpenFileBrowser,
    OpenSettings,
    OpenEpisodeBrowser(FranchiseId),
    CloseModal,
    ModalSelect(usize),

    // State sync (from UiActor, not user input)
    StateUpdated,
}
```

---

## Message Flow

### Elm Cycle

```
State Update --> Props --> Component.view() --> Render
                              |
                         User Input
                              |
                         Component.on()
                              |
                           Msg
                              |
                         update()
                              |
                    UserAction (to main loop)
                         or
                    internal state change
```

### The update() Function

The `update()` function in the UiActor processes messages from components:

```rust
fn update(&mut self, msg: Msg) -> Option<UserAction> {
    match msg {
        Msg::SendChat(text) => Some(UserAction::SendChat(text)),
        Msg::PlaySelected(idx) => Some(UserAction::SetNowPlaying(idx)),
        Msg::FocusNext => {
            self.app.active_mut().blur();
            self.focus_ring.next();
            self.app.active_mut().focus();
            None
        }
        Msg::StateUpdated => {
            self.update_all_props();
            None
        }
        // ...
    }
}
```

Some messages produce `UserAction`s that leave the UI actor (to become CRDT
ops or player commands). Others are handled internally (focus changes, modal
state, scroll position).

---

## Integration with Actor System

The UiActor bridges tui-realm and the actor system:

```
                  +----------------------------------+
                  |            UiActor               |
                  |                                  |
 StateUpdate ---->|  CrdtSnapshot -> component Props |
                  |                                  |
 TerminalEvent -->|  crossterm Event -> tui-realm    |
                  |  Event -> Component.on() -> Msg  |
                  |  -> update() -> UserAction       |----> Main Loop
                  |                                  |
 PlayerStatus --->|  position/state -> PlayerStatus  |
                  |  component Props                 |
                  +----------------------------------+
```

### State to Props Mapping

When the UiActor receives a `StateUpdate(CrdtSnapshot)`, it maps the
snapshot data to component props:

- **ChatPane**: snapshot.chat -> list of formatted message lines
- **SubtitlePane**: rolling log of `SubtitleLine` events from the PlayerActor
  (local-only; not part of the snapshot)
- **SeriesPane**: snapshot.anidb_metadata + snapshot.series_relations + local
  watch history -> franchise list (Recent/All modes);
  snapshot.list_entries + snapshot.list_next_ep -> grouped List entries
  (List mode)
- **UsersPane**: snapshot.series_preferences + snapshot.manual_overrides
  + snapshot.file_availability + peer presence/roles -> colored user list,
  with departed users and seeders on separate dim lines
- **PlaylistPane**: snapshot.playlist (sorted by position) + watched flags ->
  list items with colors based on availability, watched entries muted
- **PlayerStatus**: snapshot.now_playing + player position -> progress bar

This mapping is a pure function (presence and subtitle data arrive as
explicit inputs alongside the snapshot), making it testable independently.

### Non-snapshot inputs and the hashing overlay

Besides snapshots and terminal events, the bridge loop feeds `Ui`
local-only inputs through `UiInput`: `Subtitle(String)` (the rolling
sub-text log), `Hashing { filename, done_bytes, total_bytes,
finished }` (playlist-add hash progress), and `SearchResults { query,
results }` (AniDB name-search answers, routed to the search modal if
it is open; the modal drops results for superseded queries). The
hashing rows render as a centered overlay drawn on top of everything —
design.md's no-silent-work rule — but the overlay is *not* in the
modal stack: it captures no input, so chat and navigation keep working
while files hash. `finished` removes a row; the overlay disappears
when no hashes remain.

---

## Keybinding Bar

The keybinding bar is **derived** from the currently focused component and
any active modal. It is not manually maintained.

Each component declares its keybindings as metadata:

```rust
struct KeybindingInfo {
    key: &'static str,    // "Enter", "Tab", "Ctrl-C"
    label: &'static str,  // "Send", "Next pane", "Quit"
}
```

Each pane (and modal) exposes `keybindings()`; the dispatcher rebuilds
the `KeyBar` items from the focused component (or topmost modal) plus
global bindings (Tab, Ctrl-C) after every event. When focus changes or a
modal opens, the bar updates automatically.

---

## Modals

Modals are handled as tui-realm components that are mounted/unmounted
dynamically. When a modal is active:

1. The modal component receives focus
2. Background components are rendered but don't receive input
3. The modal is rendered as a centered overlay
4. Closing the modal restores focus to the previous component

Modal types:
- **FileBrowser**: Navigate media roots, select file
- **Settings**: First-run and later configuration
- **EpisodeBrowser**: Browse franchise seasons/episodes
- **ListEntryEdit**: Edit a List entry's fields (status, notes, next_ep, ...)
- **AniDbSearch**: Link a List entry to an AniDB series (`l` in List
  mode). Pre-searches for the entry's name; the search runs server-side
  over the anime-titles dump (not the rate-limited UDP API), results
  arrive asynchronously as a `UiInput`. Enter on fresh results links;
  editing the query re-arms search

Unlike the prototype (which used blocking sub-loops for modals), the main
event loop continues running while modals are open. Network messages, player
events, and sync updates are all processed normally.

Modals are a stack (`Vec<Modal>`): the settings screen pushes the
directory picker on top of itself for adding media roots, and the picked
directory routes back to the settings modal underneath.

---

## Testing

### Snapshot Tests (insta)

Components (and the whole `Ui`) render to a ratatui `TestBackend` buffer
and snapshot-test via `tuirealm::testing::buffer_to_string`. Whole-app
tests in `dessplay/tests/ui_app.rs` go further: scripted key sequences
through the real dispatcher, asserting on the produced `UserAction`s and
locator-style on the rendered buffer (`screen.contains("kim [ready]")`),
plus full-layout insta snapshots. Cross-client scenarios live in
`dessplay-rendezvous/tests/ui.rs` on the multi-client harness: keys into
one client's `Ui` propagate through the real server and render on
another client's buffer.

Component-level rendering can also be tested directly:

```rust
#[test]
fn test_playlist_pane_rendering() {
    let mut component = PlaylistPane::new(test_playlist_props());
    let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 20));
    let mut frame = Frame::new(&mut buffer);
    component.view(&mut frame, frame.area());
    insta::assert_snapshot!(buffer_to_string(&buffer));
}
```

### Message Tests

Test that components produce correct messages for given inputs:

```rust
#[test]
fn test_playlist_enter_plays_selected() {
    let mut component = PlaylistPane::new(test_playlist_props());
    let msg = component.on(Event::Keyboard(key_event(KeyCode::Enter)));
    assert_eq!(msg, Some(Msg::PlaySelected(0)));
}
```

### Update Tests

Test that the update function maps messages to correct user actions:

```rust
#[test]
fn test_send_chat_produces_action() {
    let mut ui = UiActor::new_for_test();
    let action = ui.update(Msg::SendChat("hello".into()));
    assert_eq!(action, Some(UserAction::SendChat("hello".into())));
}
```

### What Tests Do NOT Cover

Application logic. That's the job of SyncActor and AppState tests. The UI
tests verify rendering and input routing, not CRDT behavior.

---

## Web Renderer (Future)

The CRDT state and business logic live in `dessplay-core`, independent of
any UI framework. A future web UI could:

1. Compile `dessplay-core` to WASM
2. Subscribe to `CrdtSnapshot` updates from the NetworkActor (via WebSocket
   or WebTransport to the server)
3. Map snapshots to a web component framework (React, Leptos, etc.)
4. Use the same semantic color scheme and layout proportions

The tui-realm components are terminal-specific, but the state-to-props
mapping logic (snapshot -> display data) could be shared.
