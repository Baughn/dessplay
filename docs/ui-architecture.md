# UI Architecture

Last updated: 2026-07-20

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
- **ratatui ecosystem**: Can use any ratatui widget inside a tui-realm
  component.
- **Test helpers**: `tuirealm::testing` renders components to strings for
  insta snapshots.

We do **not** use `tui-realm-stdlib`: its `Input` was replaced by our own
[`LineBuffer`](#shared-widgets) after its horizontal-scroll bookkeeping
required repeated workarounds (dependency dropped 2026-07-02).

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
+-- Subtitle log (rolling sub-text; Off / Intermixed-into-chat / Separate pane)
+-- SeriesPane (SelectableList, three modes: Recent / All / The List)
+-- UsersPane (styled list; focusable for the Away action)
+-- PlaylistPane (SelectableList with actions)
+-- PlayerStatus (progress bar + info)
+-- KeybindingBar (derived from active focus)
```

The production shell detects terminal color depth once during setup through
crossterm plus standard `COLORTERM`/`*-direct` hints and injects `Limited` or
`TrueColor` into this same synchronous `Ui`. Rendering stays
capability-independent until the completed frame: on a
true-color terminal the theme layer gives every cell the explicit dark
background and maps semantic foregrounds to RGB, so panes, modals, and passive
overlays cannot drift onto different schemes. The same completed-frame pass
materializes `DIM` as the theme's muted RGB foreground and removes only that
modifier before crossterm output; this avoids emulator-specific SGR 2 behavior
while retaining combinations such as dim-plus-italic or selected-row reverse.
On a limited terminal the pass is a strict no-op, preserving both the
terminal's configured theme and native dim attribute. Tests inject the
capability directly rather than consulting the real terminal.

### Shared Widgets

Every pane and modal is built on the interaction primitives in
`ui/widgets/`, each implemented exactly once. This is load-bearing
architecture, not tidiness: "not all the text fields work the same way"
was a recurring bug class (word navigation and the scroll-offset reset
each landed in the chat input and silently missed the modal fields).
A behavior that exists in one place cannot drift.

| Widget | Job | Guarantee |
|--------|-----|-----------|
| `LineBuffer` / `TextField` (`widgets/line.rs`) | The one line editor: text, cursor, horizontal scroll as pure state; `TextField` adds the bordered box, placeholder, cursor cell | The full editing vocabulary (word motion via Ctrl/Alt-arrows and Alt-b/f, word kill via Ctrl-W and Ctrl/Alt-Backspace, Ctrl-A/E, Home/End) works in **every** field — chat input, modal field editors, the series filter. Scroll invariants (`offset <= cursor <= len`, reset on set/clear) are property-tested; the "field renders from a stale column" bug class is unrepresentable |
| `ListCursor` (`widgets/list.rs`) | The one selection cursor + shared bordered/body list renders | Up/Down/PgUp/PgDn, edge clamping, and cursor-centered viewports behave identically in every selectable list (panes, browsers, forms, search results); selection highlighting remains separate from the scroll target so unfocused panes can retain context |
| `Form` / `FormModel` (`widgets/form.rs`) | Field modals as typed data: models project semantic row IDs plus text / secret / toggle / choice / read-only / action controls, optional non-selectable spacing, and accept `FormEdit` at one mutation boundary. Form owns semantic selection, scrolling, masked editing, validation errors, category chrome hooks, and the save triple (capital `S`, fixed `[Save]`, unadvertised Ctrl-S alias) | Display order is not identity: insertion/reorder or visual spacing cannot retarget a field or active editor. Settings layers tabs over the same Form used by the typed List-entry editor |
| `Keymap` (`widgets/keymap.rs`) | Bindings as data: (pattern, bar entry, action method), one table per component or mode | The keybinding bar and the dispatch derive from the same table — a key shown in the bar always dispatches; a dispatched key is advertised or deliberately hidden. Actions return `None` to *decline* (guards), letting the event fall through to the structural layers |
| `widgets/keys.rs` | Key-event matchers | The terminal-compatibility policy lives in one place: bare letters over Ctrl-letters (Ctrl-J == LF, Ctrl-M == Enter, Ctrl-S == XOFF without the enhanced keyboard protocol); Ctrl *and* Alt accepted for word ops (macOS terminals send Alt); `.contains` matching for kitty's extra modifier bits |
| `table_row` / `Cell` (`widgets/table.rs`) | The one table row: a flexible name cell (styled spans, truncated with `…`) plus fixed-width columns, all in display cells (CJK-aware) | Columns never drift with content: whatever the name's length or script, every fixed cell starts at the same column (property-tested) — used by the playlist's `temp`/watch-state columns and The List's episode/watchers spreadsheet; "a long filename shoved the tags off the pane" is unrepresentable |

Event routing inside a component is layered, most-specific first:

```
typed chars (text fields / filters)  ->  ListCursor::nav (lists)
    ->  Keymap::dispatch (component keys)  ->  LineBuffer::edit (editing fall-through)
```

The widgets are pure state machines — events in, messages out, rendering
a separate function over their state — which is what keeps them portable
to a future non-terminal renderer (see [Web Renderer](#web-renderer-future)).

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
    CycleSeriesMode,            // Recent -> All -> The List (m)
    ToggleSeriesSort,           // All mode (s)
    SeriesFilterChanged,        // filter text changed (/ to start, Recent / All)
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
    MapSelected,                // manual file mapping (M)

    // Player
    SeekTo(f64),

    // Navigation
    FocusNext,
    CycleSubtitleMode,
    Quit,

    // Modals
    OpenFileBrowser,
    OpenSettings,
    OpenEpisodeBrowser(FranchiseId),
    CloseModal,
    ModalSelect(usize),

    // State sync (from the UI shell, not user input)
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

The `update()` function in `Ui` processes messages from components:

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

The synchronous `Ui` dispatcher and its production shell bridge tui-realm
components to the rest of the application:

```
                  +----------------------------------+
                  |       Ui + production shell      |
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

When `Ui` receives a `StateUpdate(CrdtSnapshot)`, it maps the
snapshot data to component props:

- **ChatPane**: snapshot.chat -> list of formatted message lines. It is also
  fed the online-username set (interactive peers, present or lost; derived by
  `props::chat_usernames`) and the local username: the former drives `Tab`
  username-completion in the input and mention highlighting in the log, the
  latter additionally reverses mentions of *your own* name. `Tab` completion
  is tried in the dispatcher's global `Tab` handler before pane-cycling --
  `ChatPane::try_tab_complete` returns whether it consumed the key (it does
  only when the trailing word is a prefix of some username), so `Tab` still
  cycles panes whenever completion doesn't apply.
- **Subtitle log**: rolling log of subtitle lines from the PlayerActor,
  each stamped with the in-video position (displayed timestamp), a
  wall-clock arrival (chat interleave key), and an optional ASS speaker.
  Local-only; not part of the snapshot. Surfaced per `subtitle_mode`: Off,
  Intermixed (folded into the chat lines via `props::subtitle_line`,
  ordered by arrival, uniformly dim), or a Separate pane that splits the
  ChatPane area -- there lines are shown newest-first and colored by speaker
  identity. The persisted speaker-name toggle defaults off; when enabled, a
  named cue is formatted as `Name: dialogue` in both Intermixed and Separate
  modes by one shared display helper. `SpeakerColors` tracks named
  speakers in an inclusive rolling five-minute wall-clock window, advancing
  on subtitle arrivals, the explicit UI snapshot clock, and a one-second
  production-shell clock tick during otherwise quiet scenes. Active slot
  assignments are unique and stable; expired slots are recycled. Backward
  clock corrections never rewind the window.

  A true-color slot extends a cached, deterministic HSLuv palette by choosing
  the candidate with the greatest minimum CIEDE2000 distance from prior RGB8
  colors. Old slots therefore remain stable and there is no application cap;
  the explicit dark frame background gives the generator a known contrast
  target. A limited terminal preserves the prior
  deterministic speaker-name hash into the finite ten-color application/user
  palette, where collisions can occur. Above that capacity, the persisted
  Playback setting chooses continued hashing (default/backward-compatible) or
  uniform dim text for every speaker until the active set is within capacity
  again. The existing speaker-colors master toggle takes precedence and makes
  all separate-pane lines uniformly dim; Intermixed is always dim.
- **SeriesPane**: snapshot.anidb_metadata + snapshot.series_relations + local
  watch history -> franchise list (Recent/All modes). Recent shows only
  *watched* franchises (recency-keyed), newest first; a `/`-initiated filter
  string (held in the component, applied in `props::franchise_rows`) narrows
  by title and lifts the watched-only default. snapshot.list_entries +
  snapshot.list_next_ep -> grouped List entries
  (List mode)
- **UsersPane**: snapshot.series_preferences + snapshot.manual_overrides
  + snapshot.file_availability + peer presence/roles -> colored user list,
  with departed users and seeders on separate dim lines
- **PlaylistPane**: snapshot.playlist (sorted by position) + watched flags ->
  list items with colors based on availability, watched entries muted. Like
  every selectable list it uses the shared cursor-centered renderer, but its
  unfocused scroll target is the now-playing row rather than its stored
  cursor.
- **PlayerStatus**: snapshot.now_playing + player position -> progress bar

This mapping is a pure function (presence and subtitle data arrive as
explicit inputs alongside the snapshot), making it testable independently.

### Non-snapshot inputs and the hashing overlay

Besides snapshots and terminal events, the bridge loop feeds `Ui`
local-only inputs through `UiInput`: `Subtitle { text, speaker,
video_millis, arrival_millis }` (the rolling subtitle log), `Hashing { filename, done_bytes, total_bytes,
finished }` (playlist-add hash progress), and `SearchResults { query,
results }` (AniDB name-search answers, routed to the search modal if
it is open; the modal drops results for superseded queries). The Nyaa
workflow similarly uses `NyaaResults` plus local import-progress inputs;
its pending-import map feeds both a passive add-progress overlay and the
modal's cancellable active list. None of this is snapshot or replicated
state. Progress rows render as a centered overlay drawn on top of everything —
design.md's no-silent-work rule — but the overlay is *not* in the
modal stack: it captures no input, so chat and navigation keep working
while files hash or download. The Nyaa modal replaces the passive overlay
while open so its cancellation controls remain visible.

---

## Keybinding Bar

The keybinding bar is **derived** from the currently focused component and
any active modal. It is not manually maintained — and per component it is
derived from the same `Keymap` table that dispatches the keys (or from
the `Form` for form modals), so the bar cannot claim a binding that does
not exist. Structural entries (list navigation, "type to filter") are
appended by the component exactly when the corresponding shared widget is
in play.

Each pane (and modal) exposes `keybindings()`; the dispatcher rebuilds
the `KeyBar` items from the focused component (or topmost modal) plus
global bindings (Tab, Ctrl-C) after every event. When focus changes or a
modal opens, the bar updates automatically.

---

## Mouse Input

Mouse events route by **position, not focus**, so they are handled at the
dispatcher level (`Ui::handle_mouse`), never inside a component's `on()`
— a pane cannot know where it was drawn. `draw` records the four pane
rectangles (`PaneRects`; the chat entry spans the whole left column) and
the handler hit-tests against them:

- **Left-click**: selects the clicked row *then* focuses the pane
  (mirroring the Tab path: `sync_focus_attr` + `refresh_keybar`). Order
  matters — the viewport the user clicked on was rendered under the
  *pre-click* focus, and each pane's `click()` method reproduces that
  viewport from its own state (the shared `clicked_index` in
  `widgets/list.rs` inverts the cursor-centered offset; the playlist's
  unfocused center is the now-playing row). Non-selectable rows (the
  seeders line) and border cells are misses.
- **Wheel**: scrolls the pane under the pointer *without* touching focus
  — the chat scrolls its log a few lines per tick, list panes move their
  cursor like Up/Down.
- **Modals**: while any modal is open, mouse events are ignored entirely
  (modals capture all input, and none of them speak mouse yet).

The production shell enables crossterm mouse capture at setup (non-fatal
if the terminal refuses; the adapter's `restore()` disables it on exit)
and the input thread forwards only left-click and wheel events — capture
also reports every motion, and each forwarded event costs a full redraw.
Tests inject `Event::Mouse` through the same `Ui::handle` as keys; the
first `draw` must happen before a click can land (zero rects miss).

---

## Modals

Modals are handled as tui-realm components that are mounted/unmounted
dynamically. When a modal is active:

1. The modal component receives focus
2. Background components are rendered but don't receive input
3. The modal is rendered as a centered overlay
4. Closing the modal restores focus to the previous component

Modal types:
- **FileBrowser**: Navigate media roots, select a file (playlist add or
  manual map), with recursive type-to-search over the library index.
  Opening it is a **round trip**: the UI thread has no storage access,
  so `AddFileAfter`/`MapFile` emit `UserAction::Browse` and the main
  loop answers with `UiInput::Browse` carrying the library listing
  (lean `(path, hash)` pairs), the personally-watched hashes, and the
  mapping browser's per-series start directory; `Ui::open_file_browser`
  unions in the group's watched flags from the snapshot and pushes the
  modal. Fetch-on-open keeps the data fresh with nothing to invalidate,
  and keeps the per-tick `UiSnapshot` lean (the library index can be
  large)
- **Settings**: First-run and later configuration, split into Account,
  Playback, Files, and IRC tabs. Left/Right changes category and each category
  remembers its semantic-row selection. Missing-required markers and the
  global Save hint derive from one validation result. The player row is a
  deliberately non-functional WIP placeholder. Playback also holds the
  opt-in **Speaker names** toggle, speaker-colors master toggle, and **Color overflow** choice; the latter is
  annotated `limited-color terminals only` because true-color allocation has
  no application cap. All other lifecycle hints reflect the current
  session-loop behavior. Header, category notes, and Save stay fixed while
  large media-root lists scroll. Files includes the default-on **Archive
  subdirectory** toggle; the UI carries its current value in each archive
  action so the file actor receives a complete destination policy.
- **EpisodeBrowser**: Browse franchise seasons/episodes
- **ListEntryEdit**: Edit a List entry's fields (status, notes, next_ep, ...)
- **AniDbSearch**: Link a List entry to an AniDB series (`l` in List
  mode). Pre-searches for the entry's name; the search runs server-side
  over the anime-titles dump (not the rate-limited UDP API), results
  arrive asynchronously as a `UiInput`. Enter on fresh results links;
  editing the query re-arms search
- **NyaaSearch**: Playlist `n`; query/results mode lists inspected single-file
  torrents, while reopening during background imports defaults to an active
  list with `d` cancel and `s` new search. Selection closes the modal so the
  rest of the TUI remains usable during the download.

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
    let mut ui = Ui::new_for_test();
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
mapping logic (snapshot -> display data) could be shared — and the
[shared widgets](#shared-widgets) (line editing, selection, forms,
keymaps) are pure state machines whose interaction logic ports as-is;
only their `render` functions are ratatui-bound.
