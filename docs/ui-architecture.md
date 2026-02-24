# Declarative UI Architecture
Last updated: 2026-02-24

## Table of Contents

1. [Target Design](#target-design)
2. [Design Changes](#design-changes)

---

## Target Design

DessPlay's UI is defined as a **declarative specification** — a data structure
describing what to display and what inputs to accept. This specification is
produced by a pure function and consumed by a platform-specific renderer.
The same spec drives both the terminal UI (ratatui) and a future web UI.

### ViewSpec

The top-level type returned by the `view()` function. It describes the entire
screen as a tree of layout nodes, plus a modal stack rendered on top.

**Layout nodes** describe spatial arrangement:

- **HSplit / VSplit** — divide space between children at a given ratio
- **Pane** — a leaf: titled, bordered, focusable, with content and keybindings
- **Spacer** — fixed-height region (e.g. the player status bar)

**Modals** are overlays on the base layout. They have their own content and
keybindings, and they capture input — the panes underneath are visible but
inactive. Modals stack (e.g. settings → file browser on top of settings).

```rust
// Illustrative — not final API
struct ViewSpec {
    base: LayoutNode,
    modals: Vec<ModalSpec>,
    status_bar: Option<StatusBarSpec>,
}

enum LayoutNode {
    HSplit { left: Box<LayoutNode>, right: Box<LayoutNode>, ratio: f32 },
    VSplit { top: Box<LayoutNode>, bottom: Box<LayoutNode>, ratio: f32 },
    Pane(PaneSpec),
    Spacer { height: u16, content: ContentKind },
}
```

### Content Kinds

Each pane or modal contains a `ContentKind` describing its semantic content.
Renderers map these to platform-appropriate widgets.

| Kind | Description | Terminal | Web |
|------|-------------|----------|-----|
| TextLog | Scrollable list of styled lines | ratatui Paragraph | `<div>` with overflow-y |
| SelectableList | Items with selection highlight | ratatui List | `<ul>` with `.selected` class |
| TextInput | Editable text with cursor | Custom cursor rendering | `<input>` element |
| ProgressBar | Fractional progress + label | Block characters | `<progress>` element |
| Composite | Vertical stack of sub-contents | Rendered sequentially | Stacked `<div>`s |

Content refers to styled text, not raw strings:

```rust
struct StyledSpan {
    text: String,
    color: SemanticColor,  // not ANSI — Ready, Error, Muted, Accent, etc.
    bold: bool,
}
```

Semantic colors let each renderer pick appropriate values — green/red/gray
in the terminal, CSS classes on the web.

### Keybindings as Routing Metadata

Each pane and modal declares its active keybindings. A binding maps a key
(or key combo) to an action and a human-readable label:

```rust
struct Keybinding {
    key: KeyCombo,          // e.g. Ctrl('m'), Plain(Enter), Plain(Char('a'))
    label: &'static str,   // "Map file", "Add", "Send"
    action: Action,         // UiAction or AppEvent
}

struct PaneSpec {
    id: PaneId,
    title: String,
    focused: bool,
    content: ContentKind,
    bindings: Vec<Keybinding>,
}
```

This is pure routing metadata — the keybindings describe what happens, but
the state machine that determines *which* bindings are active lives in
`UiState`, not in the spec. The `view()` function reads `UiState` and emits
the appropriate bindings for the current context.

### The `view()` Function

A pure function that projects application state into a UI description:

```rust
fn view(app: &AppState, ui: &UiState) -> ViewSpec
```

No side effects, no I/O, no framework types. It reads `AppState` (playlist,
users, chat, playback) and `UiState` (focus, scroll positions, input text,
modal state) and returns a complete `ViewSpec`.

Example: when `ui.metadata_assign` is `Some(...)`, `view()` emits a modal
with content and bindings appropriate to the current step (series selection
or episode input). When it's `None`, no modal appears. The renderer doesn't
need to know about metadata assignment — it just renders what the spec says.

### Input Resolution

A pure function that resolves a keystroke against the current spec:

```rust
fn resolve_input(key: KeyEvent, spec: &ViewSpec) -> Option<Action>
```

Resolution order:
1. If modals exist, check the topmost modal's bindings
2. If no modal match, check the focused pane's bindings
3. Global bindings (Ctrl-C, Tab) are always checked

This replaces the current `input.rs` match tree. Adding a new keybinding
means adding it to the `view()` function's output for the appropriate
context — no routing code to update.

### The Keybinding Bar

The keybinding bar at the bottom of the screen is **derived** from the spec,
not manually maintained. The renderer collects all active bindings (from the
focused pane and any modal) and displays them. When focus changes or a modal
opens, the bar updates automatically.

### Renderer Interface

A renderer consumes a `ViewSpec` and produces visible output. The ratatui
renderer is the primary implementation:

```rust
fn render(spec: &ViewSpec, terminal: &mut Terminal<impl Backend>) -> Result<()>
```

It walks the layout tree, computes `Rect` positions from the split ratios,
and maps each `ContentKind` to ratatui widget calls. Modals are rendered as
centered overlays with `Clear` + border on top of the base layout.

The renderer is stateless between frames — all information comes from the
`ViewSpec`. This means any state change that produces a different `ViewSpec`
automatically produces a different screen, without explicit redraw tracking.

### Web Renderer

The same `ViewSpec` maps naturally to HTML/CSS:

| ViewSpec concept | HTML/CSS equivalent |
|------------------|---------------------|
| HSplit / VSplit | CSS Grid or Flexbox with percentage widths/heights |
| Pane (border, title) | `<section>` with border, `<header>` for title |
| Focused pane | `.focused` CSS class (highlight border) |
| Modal | Fixed-position overlay `<div>` with backdrop |
| SemanticColor | CSS custom properties (`--color-ready`, `--color-error`) |
| TextLog | `<div class="log">` with `overflow-y: auto` |
| SelectableList | `<ul>` with click handlers and `.selected` class |
| TextInput | `<input>` element, cursor handled by browser |
| Keybinding bar | Footer `<div>` rendering active bindings as `<kbd>` tags |

Input resolution works the same way — `resolve_input` is pure Rust, so the
web frontend calls it with browser keyboard events translated to `KeyEvent`.
The web renderer adds mouse/click handling that the terminal renderer ignores.

The `ViewSpec` types live in `dessplay-core` (no UI dependencies), so both
the terminal client and a future web client (e.g. via wasm) can share them.

---

## Design Changes

### Current Problems

These are the structural issues driving this redesign. See
[design.md](design.md) § TUI Layout for the existing architecture.

**Brute-force redraw flag.** The event loop sets `needs_redraw = true` in
every `select!` arm. `AppEffect::Redraw` exists but is a dead no-op —
the actual trigger is the blanket flag. Any code path that mutates state
without going through a flagged `select!` arm silently fails to redraw.
This has caused the same bug twice.

**Sub-loop modals block networking.** `run_file_browser_overlay` and
`run_settings_screen` spin their own event loops with a private
`EventStream`. While these run, the main loop is blocked — no peer
messages, no player events, no sync. Opening the file browser during
playback stops the app from receiving seek/pause commands.

**Silently dropped effects.** Several `UiAction` handlers in
`apply_main_ui_action` call `app_state.process_event()` but discard the
returned `Vec<AppEffect>`. Playlist mutations (move, remove) and metadata
assignment never get their `Sync` effects dispatched — those operations
aren't broadcast to peers.

**2200-line runner.** `runner.rs` contains the event loop, all input
routing, file hash orchestration, player dispatch, and file-browser
selection logic inline. Each new feature (hashing modal, metadata assign,
mtime tracking) adds another ad-hoc code path. It's hard to verify that
all paths set `needs_redraw` and dispatch all effects.

### What Changes

| Current | After |
|---------|-------|
| `tui/widgets/*.rs` — 11 files, each a render function taking `(Rect, &mut Buffer, data)` | Replaced by `view()` building a `ViewSpec`, plus `RatatuiRenderer` that walks it |
| `tui/layout.rs` — hardcoded `Layout::default().constraints(...)` | Layout computed from the `ViewSpec` tree's split ratios |
| `tui/input.rs` — match tree routing by `Screen` × `FocusedPane` | Replaced by `resolve_input()` walking the spec's keybinding declarations |
| `tui/runner.rs` — 2200 lines, event loop + rendering + input + effects | Slimmed to: event loop → `view()` → `render()` / `resolve_input()` → dispatch effects. Mechanical, no widget logic. |
| Sub-loop modals (`run_file_browser_overlay`, `run_settings_screen`) | Eliminated. Modals are state in `UiState`; `view()` emits them as `ModalSpec`; the main event loop keeps running. |
| `needs_redraw` flag | Eliminated. Every loop iteration calls `view()` + `render()`. The renderer (or a thin diff layer) skips work if the spec hasn't changed. |
| Keybinding bar — manually maintained per-pane strings | Derived automatically from the active bindings in the `ViewSpec` |

### What Stays

- **`app_state.rs`** — `AppState`, `AppEvent`, `AppEffect`, `PlaybackState`,
  the effect system. Completely unchanged — this is already clean.
- **`ui_state.rs`** — `UiState`, `Screen`, `FocusedPane`, `InputState`, and
  the modal state structs (`SettingsState`, `FileBrowserState`, etc.).
  Mostly unchanged. Still the state machine that determines what `view()`
  produces. Minor adjustments as modals are unified.
- **`player/`** — `Player` trait, `MpvPlayer`, `EchoFilter`, `MockPlayer`.
  Unchanged.
- **Network, storage, sync** — unchanged.
- **`UiAction` / `AppEvent` enums** — unchanged. These are the actions that
  keybindings map to.

### Migration Path

The migration can be done incrementally, one widget at a time:

1. **Define the `ViewSpec` types** in `dessplay-core` (layout, content, keybinding, styled text). No rendering yet.
2. **Write `view()`** for the main screen. Initially it can coexist with the old widgets — `view()` produces a spec, but rendering still uses the old code.
3. **Write `RatatuiRenderer`** that consumes `ViewSpec`. Switch one pane at a time (e.g. users pane first — it's the simplest).
4. **Wire `resolve_input()`** alongside the old `input.rs`. Once all panes are migrated, remove `input.rs`.
5. **Eliminate sub-loop modals** by moving file browser and settings into the `view()` + modal pattern.
6. **Slim `runner.rs`** to the mechanical event loop.

Each step is independently testable. The old and new rendering can coexist
during migration — individual panes can be switched over without breaking
the rest of the UI.
