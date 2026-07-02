# Proposal: UI componentization

Status: **ACCEPTED and implemented, 2026-07-02.** All four phases landed
(LineBuffer/TextField, ListCursor, Form, Keymap); ui-architecture.md
("Shared Widgets") is the living documentation. Decisions taken: own
LineBuffer (no `tui-input` dependency; `tui-realm-stdlib` dropped);
behavior parity plus the cheap consistency wins (word ops + offset
discipline in all fields, PgUp/PgDn in all lists); the series filter is a
full TextField. The file-browser filter (#20) and sort modes (#8) did not
land — they are now drop-ins over the shared widgets.

## Motivation

Several feature requests boil down to "not all the text fields work the same
way" (word navigation landed in chat only; the horizontal-offset bug was
fixed in chat only), and a GUI remains a future plan. The root cause is
structural: the same interaction patterns are hand-implemented at many sites,
so every fix or improvement lands at one site and silently misses the others.
This proposal extracts the patterns into shared primitives so the
discrepancies become unrepresentable.

## Findings (as of 2026-07-02)

`ui/components.rs` (2,224 lines) + `ui/modals.rs` (1,614 lines).

### Text editing: four unrelated implementations

1. **ChatPane input** (`components.rs` ~691–806) — the gold standard: word
   motion (Ctrl/Alt-arrows, Alt-b/f), kill-word (Ctrl-W, Ctrl/Alt-Backspace),
   Ctrl-A/E, Home/End, history recall, tab-completion, and the hard-won
   `display_offset` reset discipline (three separate `GoTo` workarounds with
   regression tests).
2. **`FieldEditor`** (`modals.rs` ~68–128), used by Settings, ListEdit,
   AniDbSearch — char/Backspace/Delete/arrows/Home/End only. No word motion,
   no Ctrl-W, no Ctrl-A/E. Never received the offset-reset fix: editing a
   long value (media-root path, password) can reproduce the already-fixed-once
   chat offset bug.
3. **Series filter** (`components.rs` ~1269–1312) — a bare `String` with
   push/pop; no cursor.
4. **AniDbSearchModal** — wraps FieldEditor with its own event routing.

### Selection lists: eight hand-rolled cursors

UsersPane, PlaylistPane, SeriesPane, FileBrowser, EpisodeBrowser,
SettingsModal, ListEditModal, AniDbSearch results. Four use the shared
`step`/`step_by` helpers; four reimplement Up/Down inline. Visible drift:
PgUp/PgDn works in Series/FileBrowser/EpisodeBrowser but **not** in Playlist,
Users, Settings, ListEdit, or search results. Synthetic rows (`[Add New]`,
`[Save]`, `[Add media root]`, `[Select]`, `..`) are index arithmetic
(`FIXED_FIELDS + roots.len() + 1`), each site its own off-by-one hazard.

### Forms: the whole pattern duplicated

SettingsModal and ListEditModal independently implement
`field_value(index)` / `commit(index, value)` keyed by usize constants,
Enter-means-edit/toggle/cycle dispatch, the editor-swallows-input loop, the
edit-overlay rect, and the capital-S / Ctrl-S-alias / `[Save]`-row triple
(the XOFF workaround, commented in both places). J/K reorder logic exists
twice (playlist, media roots) with slightly different guards.

### Keybinding bar can lie

Each pane maintains `keybindings()` separately from its `on()` match;
nothing forces agreement.

## Proposed primitives

1. **`LineBuffer` / `TextField`** — one owned line editor (text, cursor,
   display offset as plain data) with the full editing vocabulary, and the
   offset invariants enforced in one place (property-testable: offset ≤
   cursor ≤ len; reset on set/clear). Chat wraps it, adding history +
   completion; FieldEditor and the series filter become it. Stop driving
   `tui_realm_stdlib::Input` via repeated `perform(Cmd)` calls — the code
   already fights its offset bookkeeping in three commented workarounds.
   *(Alternative: the `tui-input` crate; decision pending.)*
2. **`SelectList`** — selection cursor + clamping + Up/Down/PgUp/PgDn +
   the render block (borders, focus color, highlight) in one component.
   Rows are tagged data (item vs. named synthetic row), so `[Save]` handling
   is a match on the selected row, not index arithmetic. Optional filter
   feature (the `/`-filter generalized: TextField + predicate) — drops into
   the file browser for free (feature request #20; #8's sort modes slot in
   the same way).
3. **`Form`** — fields declared as data (`Text{label,get,set}` / `Toggle` /
   `Cycle` / `Action`); Enter-dispatch, edit overlay, and the save-key triple
   implemented once. Settings and ListEdit shrink to declarations.
4. **Declarative keymaps** — a table of (matcher, label, action) from which
   both `on()` dispatch and `keybindings()` derive, so the keybar provably
   matches behavior; the terminal-quirk policy (bare letters over
   Ctrl-letters, XOFF avoidance) documented once.

### GUI groundwork

All four primitives are pure state machines — events in, messages out,
rendering a separate function over their state. That is the shape
ui-architecture.md already promises for the web renderer: interaction logic
ports; only the ratatui render fns stay terminal-bound.

### Landing order

Each independently shippable with parity tests:
TextField → SelectList → Form → keymaps.

## Open questions (with recommendations)

1. **Line editor**: own `LineBuffer` (recommended — no dependency, property
   tests, kills the stdlib workarounds), `tui-input` crate, or keep wrapping
   stdlib Input?
2. **Scope**: strict behavior parity, or fold in the cheap consistency wins
   (recommended — word ops + offset fix in all fields, PgUp/PgDn in all
   lists, filter in the file browser) while refactoring?
3. **Series filter**: full TextField with cursor (recommended), or keep the
   append/backspace-only feel but on the shared buffer?

## Feature-request payoff

| Request | Effect |
|---|---|
| #40/#48 input-offset bugs | fixed by construction, all fields |
| #41 Ctrl-W / word nav | inherited by all fields |
| #20 type-to-find in Add New browser | filterable SelectList drop-in |
| #8 sort file list by recency | SelectList sort modes |
| #24 playlist move keys | one keymap site to change |
| #19 GUI | interaction layer becomes framework-agnostic |
