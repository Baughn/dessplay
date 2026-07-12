# Proposal: Categorised, declarative settings screen

Status: **ACCEPTED and implemented, 2026-07-12.** Decisions: category tabs;
include the upload-limit control; include the persisted player choice as a
visibly WIP, non-functional placeholder. The implementation landed as
`plan.md` Phase 23. Long media-root paths retain their distinguishing suffix
when clipped; header, notes, and Save remain fixed around the scrollable rows.

Post-implementation extension: Phase 24 added a fourth Playback row, **Color
overflow**, for the limited-terminal subtitle-speaker fallback. It is a
persisted local display setting and does not change the declarative form or
category decisions below.

## Summary

Replace the settings modal's single mixed list with four categories:

1. **Account & connection**
2. **Playback & display**
3. **Files & transfers**
4. **IRC bridge**

Categories are tabs across the top of the modal. Left/Right changes category;
Up/Down moves between that category's controls. The save action remains
global and atomic: capital `S`, Ctrl-S where the terminal delivers it, and a
visible `[Save]` row all save the complete working copy.

At the same time, replace the settings form's parallel integer-indexed
render/activate/commit implementations with typed rows declared as data. A
row should have one semantic identity, control kind, displayed value, and
effect annotation. The shared `Form` continues to own editing, selection,
save gating, and rendering.

This is a local UI refactor. It does not change the settings schema, wire
protocol, CRDT state, or command-line override rules.

## Motivation

The screen is now a flat list of unrelated concerns. Identity, rendezvous
credentials, subtitle presentation, cache policy, peer downloads, IRC, and
media roots all compete at the same visual level. IRC's four controls are
particularly dominant, while the subtitle speaker-colour control was
appended after IRC because inserting it beside `Subtitles` would have
renumbered every following field constant.

That awkward ordering reflects a code problem. A setting's position and
behaviour are currently spread across:

- thirteen `FIELD_*: usize` constants and `FIXED_FIELDS`;
- `rows()`, which establishes the actual display order;
- `activate()`, which decides whether the same integer edits, toggles, or
  cycles;
- `commit()`, which maps editable integers back to settings fields;
- media-root index arithmetic in `add_root_index()` and `on_char()`; and
- tests which navigate by repeating Down or selecting `FIXED_FIELDS + n`.

The shared `Form` extracted in the UI-componentisation work correctly owns
the behavioural mechanics, but it stopped short of the earlier proposal's
goal of declaring fields as data. Adding or moving a settings row can still
make its label, activation, and commit target disagree.

The current screen also exposes a specification mismatch worth resolving as
part of the cleanup: `upload_limit` is persisted and used at startup, and
the design says it is editable here, but the modal has no row for it.
`player` is likewise persisted and documented as a setting, but the client
always constructs the mpv backend and VLC remains an open scope decision.

## Proposed experience

### Layout and navigation

The modal keeps its current centred overlay and working-copy semantics. Its
content becomes:

```text
+ Settings -- Account & connection -------------------------------------+
| [Account !] [Playback] [Files !] [IRC]                                 |
|                                                                        |
| Username              svein                                           |
| Server                dessplay.brage.info              next restart   |
| Password              ********                        next restart   |
| Ready on startup      no                                               |
|                                                                        |
| [Save] -- needs a media root                                           |
+------------------------------------------------------------------------+
  <-/-> Category | Up/Down Field | Enter Edit/Toggle | S Save | Esc Cancel
```

- The compact tab captions are `Account`, `Playback`, `Files`, and `IRC`;
  the modal title spells out the selected category's full name.
- Left/Right changes category and Up/Down/PgUp/PgDn navigates fields.
- Each category remembers its selected row while the modal is open.
- A `!` on a category means it contains a missing required value. This makes
  first-run setup discoverable even when the missing media root is on a
  different tab.
- `[Save]` is present at the bottom of every category. Its existing
  `needs ...` explanation remains the single source of truth for validation.
- Values continue to be edited in the shared `TextField` overlay. Passwords
  remain masked in the form and no setting contents are logged.
- Rows with special application timing have a dim, right-aligned annotation
  such as `next restart` or `reconnects IRC`. Timing is metadata on the row,
  not hand-written into its formatted label.
- The modal may scroll within a category when there are many media roots;
  the tabs and save row remain visible.

Top-level tabs are preferable to section headers in one list. Headers would
make scanning somewhat better, but four extra rows, the missing upload-limit
row, and an unbounded number of media roots would immediately push the form
past the common 100x30 layout. Tabs make the categories real without adding
a second focus region or a nested menu.

### Categories

#### Account & connection

- Username
- Server (`next restart`)
- Password (`next restart`)
- Ready on startup

The username keeps the existing runtime-override lock behaviour: a one-off
`--username` remains separate from the persisted value displayed and saved
by the modal.

#### Playback & display

- Player: mpv / VLC (`WIP -- selection is not applied`)
- Subtitle display: Off / Intermixed / Separate
- Subtitle speaker colours

Speaker colours remain configurable in every subtitle mode, but render dim
with the hint `separate pane only` unless Separate is selected. This permits
preconfiguration without implying that the toggle currently has an effect.

Expose the persisted `player` field as an explicit placeholder. It cycles
between mpv and VLC and is saved, but the row is dimly annotated
`WIP -- not applied`; the composition root continues to construct mpv
regardless of the selected value. This is intentionally a visible promise,
not a claim that VLC works. When backend selection is implemented, remove the
annotation and make it a normal `next restart` control.

#### Files & transfers

- Media roots, with the first marked `download target`
- `[Add media root]`
- Cache retention
- Auto-download
- BitTorrent downloads (`next restart`)
- Upload limit (`next restart`)

A blank, non-selectable line after `[Add media root]` visually separates root
management from transfer policy.

Media-root behaviour stays unchanged: Enter on Add opens the stacked
directory picker; `d` removes a selected root; `J`/`K` reorders roots and
the cursor follows the moved root. Roots retain the persistable/runtime split
which prevents a `--media-root` override from leaking into storage.

Add the missing upload-limit editor because the setting is already persisted,
used by peer and torrent uploaders, and promised by the design. Accept
human-readable byte rates (`500 KiB/s`, `2 MiB/s`) plus `unlimited`; store the
existing `Option<u64>` bytes-per-second representation. Invalid text remains
in the editor with a concise validation message rather than silently becoming
unlimited.

#### IRC bridge

- IRC bridge enabled
- IRC server
- TLS
- Channel
- A dim warning: `IRC is public; bridged chat leaves the encrypted group.`

The server, TLS, and channel remain editable while the bridge is disabled so
the user can configure them before enabling it. They render dim when dormant.
Saving any changed IRC field keeps the existing live `Reconfigure` behaviour;
unrelated settings saves must not reconnect IRC.

### Application timing

The UI should accurately distinguish when saved values take effect:

| Effect | Settings |
|---|---|
| Immediate in the UI | Subtitle display, speaker colours |
| Live actor reconfiguration | Media roots, cache retention, auto-download, IRC fields |
| Next launch | Server, password, ready-on-startup default, BitTorrent, upload limit |

Username changes keep today's established identity-lock rules. A normal,
unlocked session adopts the saved name as it does now; a runtime
`--username` override remains authoritative for that process.

This proposal does not add live reconfiguration for startup-owned settings.
The row annotations expose the current lifecycle rather than quietly
changing it.

## Declarative form model

### Stable row identities

Replace numeric field constants with domain types along these lines:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
enum SettingsCategory {
    Account,
    Playback,
    Files,
    Irc,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SettingId {
    Username,
    Server,
    Password,
    ReadyOnStartup,
    Player,
    SubtitleMode,
    SubtitleSpeakerColors,
    MediaRoot(PathBuf),
    AddMediaRoot,
    CacheRetention,
    AutoDownload,
    TorrentEnabled,
    UploadLimit,
    IrcEnabled,
    IrcServer,
    IrcTls,
    IrcChannel,
}
```

Media-root paths are unique in the working copy, so the path is a stable row
identity while roots move. If future editing permits duplicates or changes a
path in place, replace it with a modal-local `RootId` without affecting the
form API.

### Rows as data

The shared form layer should render a typed descriptor rather than an opaque
`Line`:

```rust
struct FormRow<Id> {
    id: Id,
    label: &'static str,
    control: FormControl,
    tone: RowTone,
    annotation: Option<&'static str>,
}

enum FormControl {
    Text { value: String },
    Secret { value: String },
    Toggle { value: bool },
    Choice { value: String },
    Action { label: &'static str },
}
```

`Form` derives display, masking, and Enter behaviour from `control`. It tracks
selection by row identity, converting to a render index only at the widget
boundary. Reordering a root therefore needs no `FIXED_FIELDS + target`
calculation or `MoveTo(usize)` response.

`SettingsDraft { settings, roots }` supplies `rows(category)` as a pure
projection and accepts semantic edits in one place:

```rust
fn apply(&mut self, id: &SettingId, edit: FormEdit) -> Result<FormEffect, ValidationError>;
```

There is still necessarily a match which maps `SettingId` to the typed
`Settings` field. The improvement is that it is the sole mutation boundary;
display order no longer defines identity, and edit/toggle/cycle mechanics no
longer require separate index matches.

The shared `FormModel` API should move from `usize` callbacks to an associated
`RowId`. Migrate `ListEditForm` to a small `ListField` enum in the same change.
That keeps the shared primitive genuinely typed and completes the declarative
form direction already recorded in `ui-architecture.md`, rather than adding a
settings-only abstraction beside it.

### Separation of concerns

- `SettingsDraft`: typed working values, validation, category membership,
  and semantic edits.
- `Form`: cursor, editor, control activation, save paths, scrolling, and
  field rendering.
- `SettingsModal`: active category, per-category cursor state, directory
  picker messages, and conversion of a successful save to `Msg`.
- `Ui::update` and the session loop: unchanged ownership of applying and
  persisting the saved settings.

This preserves the Elm flow: input produces a semantic edit, the draft is
updated, and rendering is a fresh pure projection of the draft.

## Validation and save semantics

- Keep the existing atomic working copy. Esc discards every category's
  changes; Save emits the complete `Settings` plus ordered roots.
- Keep username, password, and at least one media root as required.
- Give text fields typed validators. Server, IRC server, and IRC channel
  retain their current non-empty rule; upload limit gets explicit parsing.
- A failed field commit keeps the editor open and displays its error. It must
  not silently retain the old value, which currently makes an invalid edit
  look accepted.
- Category warning markers and the Save hint derive from the same validation
  result, so they cannot disagree.

## Testing

The implementation should preserve the project's deterministic, locator-first
TUI test style.

1. **Model tests**
   - Every category projection contains unique `SettingId`s.
   - Each declared control accepts only the matching `FormEdit` kind.
   - Upload-rate parsing and formatting round-trip, including boundary values,
     whitespace, `unlimited`, and overflow. A property test is appropriate for
     arbitrary `u64` byte rates.
   - Validation derives the correct missing-category markers and Save hint.

2. **Shared Form tests**
   - Selection is retained by semantic row identity across insert, removal,
     and reorder.
   - Category switching preserves each category's cursor.
   - Secret controls never render their value.
   - An invalid commit leaves the editor active with an error.
   - Existing capital-S, Ctrl-S alias, `[Save]`, Esc, word-editing, and page
     navigation tests continue to apply to both settings and List edit forms.

3. **Settings modal tests**
   - Scripted navigation reaches every control without numeric constants.
   - Media-root reorder/remove follows the same root after movement.
   - IRC dormant styling and public warning render correctly.
   - Startup-owned rows display `next restart`; no test should imply a live
     effect that the session loop does not implement.

4. **Whole-UI tests**
   - Replace the first-run snapshot with the categorised Account view and
     assert the Files tab carries a missing marker.
   - Add one layout snapshot per category at 100x30, with a long path and
     enough roots to exercise scrolling in Files.
   - Preserve the first-run save/adopt-username test and runtime override
     regression coverage.
   - Save from a non-Account category and assert the complete settings and
     roots are emitted.

No network simulation or CRDT convergence tests are needed: the change ends
at the existing `SaveSettings` action boundary.

## Landing plan

1. Introduce typed `FormRow<RowId>` / `FormEdit` and migrate
   `ListEditForm`, preserving behaviour.
2. Replace `SettingsForm`'s integer constants with `SettingId` and a single
   declarative category projection, preserving the current flat rendering.
3. Add category tabs, per-category cursor memory, fixed Save footer, and
   effect annotations.
4. Add upload-limit parsing and its Files & transfers row.
5. Update `design.md` (categories, navigation, field lifecycle, and the
   player reality), `ui-architecture.md` (typed/grouped Form), and
   `testing-strategy.md` if the new form property is adopted. Record the
   completed work in `plan.md`.

Each step is independently testable. Steps 1 and 2 isolate the structural
refactor from the visible redesign, making snapshot changes intentional and
easy to review.

## Acceptance criteria

- The settings modal presents the four named categories and fits cleanly at
  100x30 with the keybinding bar visible.
- First-run users can see which categories contain missing requirements and
  cannot save without username, password, and a media root.
- Every existing settings behaviour remains available, including media-root
  picker/reorder/remove, all three save paths, cancellation, password masking,
  runtime override isolation, and IRC's change-only live reconfiguration.
- Upload limit is editable and stored as the existing `Option<u64>` value.
- Player choice is visible, persisted, and unmistakably marked as a WIP
  placeholder which does not yet affect the backend.
- Startup-only controls are visibly labelled as such.
- Settings rows have semantic identities; no `FIELD_*`, `FIXED_FIELDS`, or
  settings-field `usize` dispatch remains.
- Display order, activation, and commit cannot silently drift apart.
- The shared form widget remains the only implementation of cursor,
  TextField, and save behaviour for both Settings and List edit forms.

## Decisions taken

1. Use category tabs.
2. Add the upload-limit control.
3. Show the non-functional player choice as a clearly marked WIP placeholder;
   backend completion is not part of this scope.
