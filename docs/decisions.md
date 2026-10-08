# DessPlay Decision Log

Last updated: 2026-10-08

The reasoning behind the rules in [design.md](design.md): the failure that
motivated each one, the alternatives that were rejected, and the date it
was decided. design.md states *what* the system does; this file records
*why*, so the spec stays readable and the history stays available.

Sections mirror design.md's top-level sections. Each entry names the rule
and links back to the section that states it; design.md links here with
`(why: …)` pointers where a rule's reason is non-obvious.

**Workflow rule:** when a design change is made for a reason worth
remembering (a bug, a review finding, a user decision), the rule goes in
design.md and the reason goes here, in the same commit. Entries are never
deleted; a superseded decision gets a note saying what replaced it.

## UI delivery policy belongs to the mailbox (2026-09-16)

**Rule:** [UI Principles](design.md#ui-principles) retain one-shot replies and
local history, coalesce pending snapshots/progress between reliable events,
and give shutdown precedence. The user explicitly chose to preserve local
IRC, system, and subtitle history alongside completions.

**Why:** The UI's 64-slot input channel used ignored `try_send` errors for both
replaceable state and irreplaceable events. A slow renderer could lose browser
and AniDB/Nyaa search answers, hash/import completion, local-copy offers, or
shutdown. Roguelike replies had a separate retry timer; image workers blocked
on delivery to avoid leaking occupied loading slots. These were individual
repairs to one shared delivery problem. A regression left the UI unread,
submitted 128 browser requests through the real bridge, then quit. It failed
before the change because required replies disappeared.

All UI producers now use one mailbox with an exhaustive delivery-policy match.
Its send API has no capacity failure, so a new call site cannot silently choose
lossy delivery for a reply. State/progress replace only updates with the same
key in the trailing replaceable segment. Reliable events are barriers: replacing
across them could expose future state to an earlier action, or move progress
past a completion. Completion always remains an event even when it shares a
payload type with progress. The UI still rejects answers to superseded searches
using the existing request identity checks.

Blocking session sends were rejected because they can stop Quit and create a
cycle with the UI's action sends. Per-feature retries were rejected because
every new input would need to rediscover the same rule. The mailbox uses a short
transport-only lock and a capacity-one wakeup channel; it adds no forwarding
task or polling timer. Application state stays owned by its actor/UI thread.
Preserving history while producers remain live means accepting an uncapped
in-memory reliable backlog when the UI stalls. Replaceable ticks do not build
an independent backlog; there is at most one update per key between events.

The audit covered session replies, file-result forwarding, local narration and
IRC/subtitles, image workers, terminal input, first-run setup, normal teardown,
and the production harnesses. Receiver closure releases payloads; shutdown
cancels queued input even if the terminal input thread retains a sender.
Teardown closes the action receiver before joining the UI, so a UI already
blocked sending an action can exit too. The independent layout-reload channel
already retries delivery and rejects stale generations; actor protocol channels
are outside the UI mailbox boundary.

Validation on macOS: all 1,574 default-gate tests and both available release
performance tests passed, as did workspace/all-target Clippy with warnings
denied. The playback/download input probe measured 28 microseconds worst-case;
the 10,000-message chat check stayed below 8 milliseconds. These are local
synthetic measurements, not guarantees for every machine. The Linux-only CPU
test was not exercised on this host.

## Larger chat history with bounded viewport work (2026-09-15)

**Rule:** [Chat](design.md#chat) retains 10,000 synced messages after compaction,
without archive backfill. [Chat layout](design.md#runtime-layout-templates)
measures around the source anchor; [inline images](design.md#inline-chat-images)
have an independent eight-slot budget for pixels and outstanding requests.

**Why:** Search makes a 100-message history unnecessarily restrictive. A release
probe with 10,000 synthetic messages found a modest 1.34 MB serialized state and
sub-millisecond matching, but about 0.55 seconds per old-history redraw: the
renderer measured every message between the tail and the search result. A source
anchor plus pending visual-row movement removes that dependency for search,
wheel/page scrolling, redraw, resize, and message refresh. The recent-chat
projection already measures just its tail. Existing first-frame backfill,
selection, spoilers, and attachment geometry remain required.

Images previously fetched and retained every URL in the log. Enlarging that log
would scale downloads and decoded memory along with it, despite the disk cache
cap. A small nearby window keeps old images available on demand without retaining
all their pixels. Loading slots survive viewport changes, preventing rapid
navigation from creating an unbounded fetch queue. Reliable result delivery is
necessary now: a dropped answer would permanently consume the bounded budget.
Disabled images release pixels immediately; outstanding results are discarded.
Failed images may retry after leaving and revisiting the working window.

A release performance regression failed before the change at 524 ms (50 ms
budget). After the change, the oldest-result search and first paint took 2.6 ms;
the worst incoming-message refresh, wheel step, or resize took 7.2 ms on the
same development machine. These synthetic timings are local measurements, not
a guarantee for every terminal or machine. Coverage also bounds layout-tree counts independently of wall-clock
speed, checks exact wheel steps and tail following, and varies image histories
and navigation before fetch completion. The image-budget property failed before
the fix with 100 startup requests. No backfill was requested; archived messages
remain archived and the larger retained history accumulates going forward.

## Anchored chat windows fill the first frame (2026-09-15)

**Rule:** [Chat layout](design.md#runtime-layout-templates) requires both the
requested source position and a viewport's worth of measured rows before
stopping, unless the beginning of history is reached.

**Why:** A terminal recording showed that synchronization did not resolve the
reported chat-search flash. All 100 recorded frames had balanced mode-2026
boundaries, but a completed frame at 3.560 seconds placed the newest search
match near the top and left most of the log blank. At 3.626 seconds, an ordinary
redraw restored the surrounding history. The renderer stopped its backward
scan as soon as it found the anchor, even when that left fewer rows than the
viewport. End-clamping then set the scroll offset to zero; the next live-tail
draw measured a full viewport. Refresh timing determined how visible this
deterministic two-frame error was.

The minimum-row requirement now applies to both branches of the measurement
loop. It covers search submission and result navigation, as well as anchors
retained through resize, history replacement, or attachment-height changes.
Ordinary offset scrolling already requested the viewport plus its offset;
recent-chat already measures a complete viewport, and the shared document
renderer explicitly backfills at the tail. A property regression failed before
the fix with 12 rendered rows in a 30-row viewport. It varies Unicode wrapping,
viewport size, search navigation, resize, and history truncation, requiring the
first frame to be full and identical to an unchanged redraw.

## Chat search pause and terminal frame boundaries (2026-09-15)

**Follow-up:** The recording diagnosed the remaining flash as an underfilled
anchored chat frame; see the entry above. Synchronization remains useful for
atomic terminal output but cannot correct an incomplete application frame.

**Rule:** [Chat search](design.md#pane-search) applies text edits after a
one-second pause, while explicit navigation submits pending edits immediately.
[Terminal frames](design.md#terminal-frame-presentation) use synchronized output
for every production draw.

**Why:** Searching immediately on each keystroke repeatedly replaced most of the
visible conversation. The user chose a full one-second pause rather than periodic
updates while typing. The UI's existing monotonic clock owns the deadline; idle
wakeups honor its remaining duration, so the final edit runs without another key
or actor message. Matching retains the applied query separately from the editor,
preventing incoming chat snapshots from accidentally submitting pending edits.
Collection pickers retain immediate filtering.

The user also reported intermittent blank/repaint flashes while navigating
results in Ghostty directly, without tmux or SSH. The shell sent frame diffs
without declaring frame boundaries, allowing terminal refresh to race with output
delivery. Ratatui's in-memory double buffers do not synchronize physical display.
[Ghostty documents this failure mode](https://ghostty.org/docs/help/synchronized-output)
and recommends DEC mode 2026. One shared draw boundary covers initial frames,
input (including result navigation and resize), and idle animation; a guard
attempts to release the mode on errors and unwinding. Output-stream regression
tests confirmed the missing markers before implementation. They verify the
protocol contract, not the appearance of the user's physical Ghostty window.

## Shared pane search (2026-09-14)

**Rule:** [Pane search](design.md#pane-search) shares one fuzzy matcher and
editor/selection controller across local searches. Collection panes share a
ranked picker; chat jumps through the conversation using that controller in
source order. Ctrl-F opens search and `/` is an alias outside text fields.

**Why:** Series and file browsing had separate substring filters, while chat,
The List, Users, Playlist, subtitles, and logs lacked find. One matching rule
makes abbreviations work everywhere and avoids per-pane editing drift. A small
linear-scan matcher suffices: whole words, substring words, then subsequences;
there is no new dependency or remote search protocol change.

The user explicitly chose contextual jumps for chat rather than ranked message
results. Chat therefore keeps chronology, a separate search editor, and the
existing stable source-anchor renderer. Collection results carry source keys,
not mutable row indices, so accepting a result after a snapshot cannot play or
edit another entry. Selection itself only navigates. Search includes collapsed
List groups and unwatched franchises; accepting an unwatched Recent result
switches to All so the result remains visible.

Ctrl-F has a distinct legacy byte (0x06), decoded as Control-f by crossterm;
it has none of the Enter/Tab/flow-control collisions behind the earlier
bare-letter policy. That policy still applies to the existing mode/sort keys.
This supersedes the old Series-only slash filter and its inline caption.

## Borrowed CRDT map traversal (2026-09-12)

**Rule:** Read-only map traversal borrows entries without constructing mutation
contexts; see [State Sync Protocol](design.md#state-sync-protocol).

**Why:** Samply profiling during real playback with about 52,000 catalog and
metadata entries attributed roughly three quarters of client CPU time to view
construction. `crdts::Map::iter()` cloned two causal clocks for every entry;
our read-only callers discarded both. A borrowed iterator removes that work
without changing the state model, merge behavior or snapshot representation.
On the copied replica, an isolated complete-view benchmark roughly halved
construction time and reduced allocations from 397,474 to 175,244 per view.
These are operation measurements, not a claim that all view costs disappear.

Three 30-second real-playback samples with the List expanded measured 16.3%
of one CPU core before, 10.2% after, and 15.9% when returning to the original
binary. Median RSS varied enough between runs that no consistent resident-memory
reduction was established; the reliable memory result is reduced allocation
traffic (104,368,055 to 40,365,815 requested bytes per view in the isolated probe).

We vendor the published `crdts` 7.3.2 with one iterator addition and explicit
lifetimes on four existing iterator signatures to keep current Rust builds
warning-free. The iterator addition is marked in `map.rs`; `Cargo.toml`
documents both kinds of local change. The user accepted
maintaining this dependency patch. It can be retired when an upstream
release supplies equivalent borrowed traversal. Caching resolved maps was
deferred: it would add invalidation obligations across writes, merges and
replica replacement, whereas this change keeps every view freshly resolved.

## Click-to-expand chat images (2026-09-11)

**Rule:** See [Inline Chat Images](design.md#inline-chat-images).

**Why:** Inline previews keep chat readable but are too small for inspecting
pictures. A mouse-only fullscreen view provides more space without another
keyboard binding. Every key dismisses it before global shortcuts run, so
closing the picture cannot accidentally quit, toggle readiness, or edit chat.
The opening press's release is ignored so a normal click leaves the viewer open.

Hit targets come from the final painted image operations, after clipping and
overlay suppression. This keeps scrolled and customized layouts aligned with
what is visible. The viewer owns a decoded source and a separate size-cached
encoding; it survives chat-window pruning and never magnifies an inline crop.
Background layout is suspended while viewing to retain the chat scroll anchor
and prevent graphics protocols or passive overlays from covering the picture.

## Installed builds retain their active compilation cache (2026-09-15)

**Rule:** See [Installed build-cache retention](design.md#installed-build-cache-retention).

**Why:** Script-managed installations should not accumulate every historical
compiler/dependency configuration. However, Dagger's machine takes twenty minutes
to build the client. A byte cap, periodic `cargo clean`, or expiry of otherwise
useful artifacts exchanges disk growth for unacceptable rebuilds. The retention
boundary is the current successful build and its complete dependency closure.

`cargo-sweep` was considered. Its age-based policy reads fingerprint access times,
which may not advance on all filesystems, and it does not track incremental
directories. It also does not acquire Cargo's build locks. Instead the launcher
captures Cargo's JSON report and invokes maintenance in the newly built client,
using its existing JSON/hash dependencies. Additional Cargo invocations remain
fresh; no Python installation, extra Cargo plugin, nightly-only flag, compiler
option change, or cold-cache migration is needed.

Cargo's report includes fresh libraries and cached build-script results, but its
binary entries name the uplifted executable rather than its hashed intermediate.
Matching both contents and modification time covers hardlinks and macOS's
clonefile copies. Verifying the retained fingerprints' dependency edges catches
a later build replacing that executable, even when the replacement was already
cached. Unknown Cargo formats fail closed. Cleanup takes both legacy and newer
Cargo locks nonblockingly and retains units changed since the build began.

Incremental directory names use rustc's own disambiguator, not Cargo's artifact
hash. A transparent rustc wrapper records the crate directories changed during
successful compilation. Concurrent invocations may record overlapping ownership;
retention uses the union for live units. Existing caches remain in place, and
missing ownership information protects a live crate's legacy directories. The
launcher does not trade twenty minutes of compilation for a smaller migration.

## Stable is a delayed bookmark, with launcher-owned selection (2026-09-09)

**Rule:** See [Launcher update tracks](design.md#launcher-update-tracks).

**Why:** Dagger's older computer should not need to rebuild for every feature or
non-critical fix. A delayed bookmark keeps everyone on one development history
while protocol changes and critical fixes still reach stable users promptly.
Release branches and backports would add maintenance without helping this goal.
The initial implementation advances stable once to ensure either track supports
the flag and control; subsequent routine commits can leave stable behind.

The installer has no selection UI. The application edits the exact file named by
the launcher, so a build launched directly cannot claim to configure an installer
it knows nothing about. This runtime capability travels with the settings working
copy but never enters SQLite. First-run and in-session saves share the same atomic
file writer; unrelated saves do not rewrite the choice.

A fast-forward pull cannot switch to an older stable revision. An explicit shallow
fetch and detached checkout handle both directions and migrate existing
single-branch clones without discarding local edits. Script replacement is checked
before either build path: otherwise a launch can use obsolete flags or build
commands after updating. Re-execution skips its second fetch to avoid update loops.
The first upgrade from the legacy launcher may still need the already-accepted
second launch before the settings control is enabled.

## Installed launcher preserves the caller's directory (2026-09-09)

**Rule:** Installation and launch preserve the caller's working directory;
see [design.md](design.md#first-launch).

**Why:** The launcher changed into the checkout for `git pull` and left
`cargo run` there, so `layout init dirname` exported into the repository.
Both the Nix and system-Cargo branches affected every relative path, including
layout validation/overrides, CSV imports, database/cache/media/socket paths,
and startup `.env` lookup. The update now changes directory only in a subshell;
Cargo receives the checkout's explicit manifest path and runs from the caller's
directory. This preserves Cargo's executable discovery and runtime environment
without hardcoding a build-output path or repairing individual CLI arguments.

## Measured dungeon inspection and summaries (2026-09-08)

**Rule:** Condition navigation and bounded wound/threat summaries consume the
same measured scenes they paint. Anatomical IDs and source fields survive row
rearrangement; guide/journal scrolling shares the document-anchor service.

**Why:** Prewrapping strings in the controller would diverge when CSS changed
padding, fields, or text wrapping. Measured line records retain treatment
identity even within an entry taller than the viewport. Summary budgets retain
whole entries and omission markers without manually assembling padded rows.
The recent journal and full journal are explicit projections with stable event
keys. Opening a 5,000-entry document at the tail instantiates only visible
entries. The larger dungeon controller is boxed in the modal enum so document
state does not inflate every modal allocation.

## Styles resolve before painting (2026-09-08)

**Rule:** Scenes and intrinsic primitives resolve terminal colors before they
paint. The final whole-buffer theme pass is removed. Authored foreground colors
override semantic dimming at every presentation boundary.

**Why:** A final pass obscured CSS precedence and needed an image-pixel exclusion
list. Resolving styles at paint time preserves authored colors and lets image
protocols bypass text conversion structurally. The old pass remains a test-only
palette oracle; a matrix covers true-color/limited foregrounds, backgrounds,
and modifiers. A new regression first demonstrated component-level DIM defeating
an explicit CSS foreground. One shared merge handles component, plain-field,
rich-span, editor, and dungeon-cell styling to prevent that class recurring.

## Page shells and intrinsic progress overlays (2026-09-08)

**Rule:** Templates own the full page/recent-chat/keybar split and work-progress
composition. Progress fill is a typed intrinsic primitive. Explicit floor cell
rounding preserves the default foreground page boundary.

**Why:** Independent page and recent-chat rectangles could diverge after file
edits. A shared scene makes their relationship editable and consistent. Existing
small-terminal and snapshot checks caught nearest rounding moving the split;
freezing the explicit extent before sibling allocation preserves the old default.
A new intrinsic overlay check also caught opposing absolute insets stretching an
auto-height modal. Removing the opposite inset for auto dimensions fixes that
class while retaining centered placement and definite modal dimensions.

## Measured health priorities and template form chrome (2026-09-08)

**Rule:** Health priority budgets measure authored metric/progress components
without wrapped-height feedback. Form entry templates own modal sizing and
footer/error appearance; controls retain only semantic identity and state.

**Why:** Padding complete strings made field rearrangement impossible and hid
reserved spacing from CSS. Intrinsic measurement preserves the existing narrow
terminal degradation while exposing metrics and playback times independently.
Keeping the marquee slice and progress fill as small terminal primitives avoids
making animation a markup language. Template modal wrappers remove the model's
percentage rectangle arithmetic, and slot-layer painting keeps editors above
their authored chrome when the form is rearranged.

## User Experience

### Reset synced state has no confirmation

**Rule:** The Settings → Account "Reset synced state" row (and `/resync`) clears the local replica and restarts the client with no confirm step; see [design.md](design.md#settings-screen).

**Why:** The shared state is losslessly recoverable from the server (the restart re-adopts the server's copy), and local-only tables (watch history, hash cache, manual mappings) are untouched. A confirm step would guard against a loss that cannot happen; the deliberate act of typing the command or choosing the row is enough.

### Bare letter keys instead of Ctrl-modified letters

**Rule:** Root reordering in Settings uses bare `J`/`K` (not Ctrl-J/Ctrl-K), the playlist pane reorders the same way, and Series-pane filtering is gated behind `/` rather than a Ctrl chord; see [design.md](design.md#settings-screen) and [design.md](design.md#adding-files-to-the-playlist).

**Why:** Ctrl-modified letters collide with control codes in terminals without the enhanced keyboard protocol: Ctrl-J is LF, Ctrl-M is Enter. Bare letters are delivered reliably everywhere. Gating the filter behind `/` also keeps the bare `m` / `s` keys live in the Series pane while no filter is being typed.

### Series pane opens on The List

**Rule:** The Series pane's three modes cycle Recent Series → All Series → The List, and the pane opens on The List by default; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** The spreadsheet view is the day-to-day "what are we watching" surface, so it is what the user should see first.

### Episode browser season tree shape and dimming

**Rule:** The Episode Browser shows a franchise's seasons as a tree: the prequel chain in order with Sequel/Prequel-chained OVAs inline, side branches indented under the season they branch from and placed chronologically; a season whose every known file is watched renders dim; the cursor opens on the first season with an unwatched file; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** OVAs that AniDB chains as Sequel/Prequel stay inline because that is how the group watched them. Dimming uses watched-ness rather than The List's held-copy rule because a season nobody advertises right now is not *done*; the library index is the durable record of what exists, so "no one is seeding it tonight" must not read as "finished".

### Episode grouping and the any-copy watched rule

**Rule:** Episodes group by AniDB episode identity; a file with no parseable episode number never merges with an adjacent one; a multi-copy episode's header is muted when any copy is watched while copies keep their own marks; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** Two files with no episode number sitting next to each other is no evidence that they are the same episode, so they are never merged on adjacency alone. A multi-copy episode counts as watched when any copy is because the group saw the episode, whichever encoding carried it; per-copy marks are kept so "which file did we actually play" is still answerable.

### Add browser opens on the selected entry's file

**Rule:** Pressing `a` in the Playlist pane opens the file browser on the selected entry's local file when it has one, else at the media roots; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** Pressing `a` on the just-watched episode is the common way to queue the next one; opening beside it puts the next episode a keypress away.

### Browser sort by newest mtime

**Rule:** `Tab` toggles the add/map browser between alphabetical and newest-mtime-first; directories always stay alphabetical; the choice is persisted; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** Files that just landed (a fresh download, a copy dropped in moments ago) float to the top under newest-mtime-first. Directories stay alphabetical because a directory has no single meaningful mtime. The choice is persisted for the same reason the All Series sort is: a sort preference is stable per user, not per session.

### Paste-to-add anchoring and in-place registration

**Rule:** A pasted single existing-file path adds it after the playlist's currently selected entry whichever pane is focused; the path is canonicalized; an out-of-root file is registered in place as a manual-mapping row; any other paste goes to the chat input; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** A drag lands wherever the cursor happens to be, and there is no use for posting a file *path* to chat, so the add is anchored to the playlist regardless of focus. Terminals produce several shapes on drag (bare, shell-escaped, quoted, percent-encoded `file://`), so each reading is tried in turn. Canonicalizing at the boundary keeps a relative or symlinked form from becoming a cwd-dependent registration. Registering an out-of-root add in place (rather than copying into the cache) makes it servable across restarts; the cost is that moving the file afterwards breaks it, exactly like a moved manual mapping.

### Control characters are stripped from pastes and inbound chat

**Rule:** Pastes drop control characters whether they land in a modal text field or the chat input; the chat display strips control characters from inbound synced and IRC-bridged lines too; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** Typing can never produce a control character, so a copied value's trailing newline must not land invisibly in a settings field, nor in a chat message whose bytes would sync to every peer's terminal. Stripping inbound lines is the defensive half: a hostile or malformed remote message must not be able to write raw escape bytes into the terminal.

### A missing file from a known series blocks

**Rule:** A playlist file that is not found locally blocks playback (state Missing) when its series is one the user has watched before; an unknown series instead sets the user Not Watching; see [design.md](design.md#file-matching).

**Why:** If you have watched this series before, you probably should have this file, so the group waits rather than silently going on without you. For a series you have never touched, the opposite default is right: nobody expects you to have it, and the placeholder plus Not Watching lets the group proceed.

### Manual mapping path canonicalization and loss handling

**Rule:** A manually mapped (or dragged-in) path is canonicalized at the boundary; a mapping whose file has vanished falls back to normal matching and resolves Missing; a loss observed mid-session prunes the durable row, while a mapping absent at startup is kept unregistered and revives if the path returns; see [design.md](design.md#file-matching).

**Why:** Canonicalizing means the durable mapping row never depends on the working directory. Falling back to Missing is the honest state: the alternative is wedging on a phantom copy. The two loss cases differ in evidence: a failed serve or load mid-session proves the file is gone, so the row is pruned; a path merely absent at startup may be an offline mount, so the row is kept and the mapping revives when the path returns.


### Commitment keyed by List entry, not AniDB id

**Rule:** Per-series watch preference is keyed by `(UserId, ListEntryId)`; AniDB linking is enrichment only. See [design.md](design.md#user-states).

**Why:** A real number of series the group watches have no AniDB entry at all (obscure OVAs, doujin work, non-Japanese content, very new simulcasts). Hanging commitment on an AniDB id would make those series impossible to commit to. See also [Series Identity](design.md#series-identity) for why the per-file derived name was rejected as a key.

### Ctrl-R never commits to Watching

**Rule:** `Ctrl-R` / "mark ready" / `/ready` clears a pause or an auto-`NotWatching` back to Maybe, never to Watching. See [design.md](design.md#user-states).

**Why:** Watching is the one state that blocks the group across the user's absence. That commitment must always be opt-in through a deliberate act (`/watch`, the watch-cycle key, or a List entry's `watchers` set), never a side effect of the reflexive "I'm ready" key.

### Away is cleared by activity, not by typing

**Rule:** Any user may mark another Away; the mark is cleared only by the marked user's client attempting to unpause or sending a chat message, not by typing. See [design.md](design.md#user-states).

**Why:** Away exists for when someone walks off without quitting and would otherwise block playback forever. Clearing on *sending* rather than *typing* lets the marked user compose a message while still marked away. There is no permission system because the app is for five trusted friends.

### Marking others not-watching is plain LWW

**Rule:** `n` on a user in the Users pane or `/skip <name>` writes that user's series preference to NotWatching as an ordinary LWW write with `set_by: Some(actor)`. See [design.md](design.md#user-states).

**Why:** This is the "Kim tool": rule on someone's commitment to a show without waiting for them to show up, or acknowledging them file-by-file. A durable preference change keeps playback unblocked for the whole series. Plain LWW (the subject's own later write overrides it) was chosen over Away's special clear-by-activity rule because a series preference is a durable opinion, not a transient presence state. `set_by` carries the setter so the narrator can attribute the write ("by Baughn").

### Playable verdict is computed by the downloader

**Rule:** The downloading client computes whether the 20% window ahead of its playback position is complete and advertises `Downloading` or `DownloadingPlayable`; no other client recomputes the gate. See [design.md](design.md#file-state).

**Why:** The downloader is the only party holding both the chunk bitmap and its own player position. Publishing the verdict as an availability variant means every client derives the same gate with no arithmetic and no shared view of the bitmap. The window is approximate on purpose: time maps to bytes proportionally, and the 20% buffer exists precisely to absorb variable bitrate.

### In-place completion needs no reload; other channels always reload

**Rule:** A download that completes at its own cache path does not reload the player; a verified copy arriving via any other channel at the same path always re-issues the load. See [design.md](design.md#file-state).

**Why:** The download assembles in place, so completion is the same inode and the player's open fd sees the full content. A browse import placing a fresh inode over the same path is a different file as far as the open fd is concerned. Path equality never implies content identity.

### Phantom EOF on a sparse partial (2026-08-21)

**Rule:** A partial's EOF report is believed only when the last known position is within a few seconds of the entry's duration; a rejection re-arms EOF reporting and seeks back to the last position observed while the file was advertised playable, never seeking the same target twice without a position tick past it. See [design.md](design.md#file-state).

**Why:** Unfetched regions of a sparse partial read as zeros, so the player can report end-of-file mid-episode. The 2026-08-21 review found the rejection path could spin. The seek target is the last position observed *while advertised playable* because that is verified data by construction: the raw last position is the very offset that produced the phantom EOF, and after a user seek it is the data-less seek target itself. The in-place completion path re-issues the Load when a rejected EOF is outstanding because mpv parked at a phantom end under `--keep-open` never fires another EOF on its own.

### Unopenable partial retry gap

**Rule:** A partial the player cannot open is not offered again until ~10% more of the file has arrived; meanwhile the client advertises plain `Downloading`. See [design.md](design.md#file-state).

**Why:** The same bytes fail the same way, and repeated failures must not loop. This is the file-level analogue of the player-process crash ladder (see [Player Lifecycle](design.md#player-lifecycle)). Advertising `Downloading` rather than `DownloadingPlayable` during the deferral is deliberate: the client holds no playable video, so it should gate rather than let the group play on without it.

### Bitrate-vs-download-speed unpause rule dropped (2026-08-17)

**Rule:** There is no download-speed-vs-bitrate half to the Downloading gate. See [design.md](design.md#file-state).

**Why:** The original rule compared download speed to the file's bitrate before allowing unpause. Since the anchored download policy the situation comes up rarely, and the group decides by watching how fast the download percentage moves. Dropped in the 2026-08-17 usage triage.

### Paused is yellow, not red

**Rule:** A Paused user renders yellow in the Users pane although they block exactly like a red row. See [design.md](design.md#ready-states-ui-display).

**Why:** Yellow says "a friend paused", red says "something is broken". Both block; the colour distinguishes the deliberate from the faulty.

### Downloads are never shadowed in the Users pane

**Rule:** A peer downloading the now-playing file always reads as Downloading, whatever its derived state; the colour (green / blue / red) carries the rest. See [design.md](design.md#ready-states-ui-display).

**Why:** An in-progress download must never be hidden behind a Paused / Away / Not Watching label. The download is the thing to watch; red on a Downloading row says they still won't be watching right now. A present Maybe user displays exactly like Ready because both gate on their file state while present; the per-series distinction is shown in the playlist's watch tag instead.

### Waiting-for OSD shares the gating derivation

**Rule:** The "Waiting for …" mpv overlay is derived from the same gating derivation as the Users pane, sits in its own `osd-overlay` slot (top-right), is shown to everyone including the blockers, and is re-applied after a relaunch. See [design.md](design.md#ready-states-ui-display).

**Why:** Deriving both from one source means the overlay and the Users pane can never disagree. A separate overlay slot from the chat OSD means chat traffic never hides the blocker line.

### Auto-NotWatching suppressed when the file is obtainable

**Rule:** A missing file from an unknown series sets Not Watching, unless a present peer advertises the file Ready, in which case the client downloads instead. See [design.md](design.md#ready-states-ui-display).

**Why:** Writing a sticky Not Watching for a file the seeder is about to serve would opt the user out of a show they could have watched. The residual race (the source's Ready not yet synced when the decision fires) is accepted: it can set Not Watching once, the Downloading display masks it, and Ctrl-R clears it.

### Download progress visible without selection

**Rule:** Every playlist entry the client is downloading shows its percentage in the Playlist pane. See [design.md](design.md#ready-states-ui-display).

**Why:** Downloads mostly happen in the background (prefetch), so progress must be visible without selecting the file. This is an instance of the "no silent long-running work" UI principle.

### Absent Maybe users do not block

**Rule:** A Maybe user blocks only while present; Lost or Departed Maybe users never block. See [design.md](design.md#playback-rules).

**Why:** We don't hold up the night for someone who isn't here and only *maybe* wanted this. Watching (committed) is the deliberate opt-in for "wait for me even if I've been gone a week."

### Playback intent is a latch

**Rule:** Playback intent is a synced `LwwCell<PlaybackIntent>` that the server forces to Paused on any Lost, on graceful quit during playback, and on EOF-advance. See [design.md](design.md#playback-rules).

**Why:** The register is the latch that keeps playback paused after a blocker leaves instead of silently auto-resuming. The server forces Paused on *any* Lost, committed or not; gating then decides whether pressing play resumes (yes for an absent Maybe user, no for a committed one until acknowledged).

### Lost-to-Departed promotion does not re-pause

**Rule:** The timeout-ladder Lost->Departed promotion does not re-force Paused; only the graceful-quit immediate departure force-pauses. See [design.md](design.md#playback-rules).

**Why:** The peer was already paused at its Lost transition 30s earlier. Re-pausing at Departed would clobber a resume the present users legitimately made during the Lost window (an absent Maybe user is non-blocking, so such a resume is valid). Graceful quit skips Lost entirely, so it is the one departure that has not already paused.

### Acknowledge is a per-file set, not an Away

**Rule:** `/ack` records `(now-playing file, absent user)` in the grow-only `acknowledged_absent` set, cleared at compaction. See [design.md](design.md#playback-rules).

**Why:** The block should re-raise on the next episode and be re-acknowledged consciously each file. Reusing the per-user Away override would persist across episodes until the user returned. The per-file scoping is why this is a dedicated set.

### Drift correction hysteresis and rate limiting

**Rule:** Slew engages at 150ms and releases at 25ms; speed updates are quantized and rate-limited to ~1/s. See [design.md](design.md#playback-rules).

**Why:** A sustained pitch-corrected 2% slew is inaudible, but each speed *transition* is a broadband click. Corrections must therefore be few and long rather than frequent and brief, which is what the wide engage/release gap and the rate limit buy.

### Resident scaletempo2 for drift slew

**Rule:** On every mpv connection, setup appends a labelled `scaletempo2` to the user audio filter chain. See [design.md](design.md#commands-sent-to-player).

**Why (2026-09-24):** The per-transition click found on 2026-07-22 comes from mpv's automatic pitch correction (autoaspeed, `filters/f_auto_filters.c` in mpv 0.41.0), not from scaletempo2's time stretching. Autoaspeed creates a new scaletempo2 when speed leaves 1.0. When speed returns to 1.0 it drains that instance with EOF and deletes it, which pads the tail with silence and truncates it. Neither splice is crossfaded, and WSOLA output is only aligned with its input to within ±20ms, so each splice clicks. Speed changes that stay away from 1.0 go to the existing instance and are smooth. A user-chain tempo filter takes the speed commands instead (`set_speed_any`), so autoaspeed stays inactive. The resident instance passes audio through near 1.0 and re-syncs exactly on entry. Doing this over IPC in setup, rather than with a launch flag, also covers `--attach-mpv` and survives a profile that sets `af`. We use `add` rather than `set` so the user's own filters are kept. Rejected: waiting for an upstream fix, since the handoff to mpv is in progress but a patch would take a release cycle to reach users. Measured by `dessplay/tests/mpv_audio.rs`: the worst splice went from -9.8dB out-of-chord energy (with about 14ms of audio dropped) to -141.6dB, which is float rounding. The hysteresis and rate limiting above predate this fix and are kept for now. They could be relaxed, but that is a separate decision.

### Invalid user authority is never followed

**Rule:** A user seek authority is followed only when that user is present and advertises the now-playing file `Ready` or `DownloadingPlayable`; otherwise it is treated like Server authority. See [design.md](design.md#playback-rules).

**Why:** A user can hold seek authority without being on the real video, e.g. a not-watching client whose player shows a placeholder, which still reports a position. Following it would freeze the whole group on its bogus position. `DownloadingPlayable` counts because a downloader's partial is the real video, and when the whole group is downloading a fresh episode they are the only valid sources there are.

### Leader fallback under Server authority

**Rule:** Under Server authority (or an invalid user authority) each client follows the furthest-ahead present peer that has the now-playing file loaded. See [design.md](design.md#playback-rules).

**Why:** The Server has no position, and it holds authority for most of an episode. Without a fallback the player would run open-loop: any initial offset (a late-starting player, a brief decode stall) would sit uncorrected for the whole episode. Following the front means laggards catch up forward with no group rewind.

### Position file tag gates leader eligibility

**Rule:** Leader election and user-authority validation both require the peer's `PlaybackPosition` to carry a `file` tag equal to now-playing, in addition to `Ready`/`DownloadingPlayable`. See [design.md](design.md#playback-rules).

**Why:** `Ready` is set on prefetch, so a peer advertises Ready for next week's episode long before playing it. Right after a now-playing transition a peer can be Ready for the new file while its position register still holds the *previous* file's sample; forward-only leader election would latch the group onto that stale value instead of starting the new file at T=0. The tag is a clock-free identity check that excludes absent users, users on a different file, and users watching a placeholder, whose positions are stale or for another file. The tag is trustworthy at its source because the player actor attributes positions to a file only after mpv's path echo confirms the load (see [Events from Player](design.md#events-from-player)).

### Resume point on load

**Rule:** Every `Load` of the real now-playing video seeks to the furthest position any user, present or not, has persisted for this file. See [design.md](design.md#playback-rules).

**Why:** Drift correction follows only *present* peers, so it cannot restart a session that ended mid-episode: with everyone gone there is no leader, and a fresh client would sit at zero under Server authority. Positions are replicated CRDT state that outlives the users who wrote them, so the furthest one is available. Furthest-ahead matches the leader rule, so whoever loads later converges on the same point. The seek goes through the crash-restore path so it is programmatic, echo-suppressed, and never a `UserSeek`.

### Manual select does not mark watched

**Rule:** Manually selecting a different playlist entry loads it paused at the start and resets seek authority, but does not mark the abandoned file watched or advance The List. See [design.md](design.md#playback-rules).

**Why:** Selecting a different file abandons the current one rather than finishing it. The pause-at-start half mirrors the EOF transition so the group presses play when ready either way.


### OSD chat lines expire individually

**Rule:** each chat line on the player OSD stays a minimum of 8 seconds and expires on its own; see [design.md](design.md#chat).

**Why:** a burst of messages must never erase an unread line. A single shared timeout for the whole overlay would let the newest message push an older, still-unread one off the screen.

### Tab completion yields to pane cycling

**Rule:** `Tab` in the chat input completes a username prefix, and otherwise keeps its normal job of cycling panes; see [design.md](design.md#chat).

**Why:** completion should be invisible until it is useful. Reserving `Tab` for completion alone would cost the pane-cycling key for every keystroke that is not a name.

### Spoilers are a display concern

**Rule:** `||spoiler||` runs sync and archive as raw text; every display surface scrambles them; only the chat pane can reveal; the scramble is seeded by message identity; see [design.md](design.md#chat).

**Why:** Discord's `||...||` syntax was chosen for familiarity. Keeping the raw text on the wire follows the same rule as CTCP actions (only the display sites decode), so no message type or schema change is needed.

The scramble replaces characters class-for-class (letters and digits keep their class; CJK, emoji, arrows, and other symbols become letters) so nothing about the original leaks through the shape of the text. Seeding from message identity rather than an RNG keeps the scramble stable across repaints and identical between the chat pane and the OSD.

The OSD and IRC deliberately have no reveal: IRC is public, logged, and one group member's primary chat surface, so a reveal affordance there would hand the spoiler to exactly the people the sender hid it from.

### Summon decides client-side and matches nicks in the IRC actor

**Rule:** `/summon` decides "IRC bridge disabled" and "everyone's here" in the client, and the IRC actor performs the nick matching and sends the ping directly as a PRIVMSG, not via `Mutation::Chat`; see [design.md](design.md#chat).

**Why:** both early-exit conditions are already known client-side, so deciding them needs no round trip. Channel membership (from NAMES/JOIN/PART/QUIT/NICK) lives in the IRC actor, so that is where the edit-distance matching of absent usernames to live nicks belongs. The ping addresses specific nicks rather than broadcasting to the group, so it is not a chat message: it is not mirrored into the local chat log and not synced. Only the outcome (who was pinged) becomes a local system line.

### /me actions ride inline as CTCP ACTION

**Rule:** `/me` is carried inline in the message text as `"\x01ACTION …\x01"`; only display sites decode it; the action phrase renders grey; see [design.md](design.md#chat).

**Why:** using the CTCP `ACTION` convention inline means no separate message type and no schema change, and the wire form forwards verbatim to IRC. Terminals have no italics, so colour (grey) is what marks the emote in the chat log.

### /resync needs no confirmation

**Rule:** `/resync` (and the Settings → Account action row) clears the local synced state and restarts the client without a confirm modal; see [design.md](design.md#chat).

**Why:** typing the command is the deliberate act, and the shared state is losslessly recoverable from the server. Local-only tables (watch history, hash cache, manual mappings) are untouched, so there is nothing to lose that a confirmation would protect.

### IRC bridge motivation

**Rule:** each interactive client optionally mirrors its own chat into a shared IRC channel and surfaces plain-IRC messages back into the chat pane; see [design.md](design.md#irc-bridge).

**Why:** DessPlay logs are unavailable when the program is not running, so the chat is gone the moment the app is closed. An IRC channel is something others can keep open or log independently of the app.

### Dess suffix stays terminal

**Rule:** the IRC nick is `[Username]Dess`, and a collision retry keeps `Dess` as the suffix (`Baughn2Dess`); see [design.md](design.md#irc-bridge).

**Why:** the suffix is how *other* bridges recognize and de-duplicate bridged messages (see the inbound `*Dess` filter). A disambiguator appended after the suffix would defeat that recognition.

### IRC spoiler mask seeding

**Rule:** outbound `||spoiler||` runs are masked at the `Mutation::Chat` tap with a static scramble seeded from a per-process message counter, never from the message text; see [design.md](design.md#irc-bridge).

**Why:** the channel is public and logged, one group member reads chat *only* there, and IRC has no reveal affordance, so raw bars would hand the spoiler to exactly the people the sender hid it from. The mask is seeded from a counter because a plaintext-derived mask would let a channel lurker confirm a guessed spoiler by recomputing it. The consequence, that the IRC letters differ from the chat/OSD rendering of the same message, is harmless: nobody cross-checks them.

### CTCP actions are never split

**Rule:** long plain IRC lines are split at the 512-byte limit, but a `/me` CTCP action is never split; see [design.md](design.md#irc-bridge).

**Why:** chunking an action would break the `\x01` framing or emit several separate emotes for one action. Leaving an over-long emote to the server's 512-byte truncation is the conventional IRC client behaviour; this is intentional.

### Inbound IRC lines are local and Dess nicks are dropped

**Rule:** messages from IRC nicks not ending in `Dess` are shown locally and never synced; messages from `*Dess` nicks are dropped; see [design.md](design.md#irc-bridge).

**Why:** each client runs its own bridge, so syncing inbound lines would duplicate them once per client. `*Dess` nicks are other bridges echoing DessPlay users who are already present via CRDT sync. The heuristic cost, that a genuine IRC user whose nick ends in "dess" (e.g. `Goddess`) is also dropped, is accepted: the actor deliberately does not hold the roster.

### IRC presence notices (2026-09-13)

**Rule:** External IRC joins and departures become local system notices, with
QUIT and KICK included, while initial rosters and bridge nicks stay quiet;
see [design.md](design.md#irc-bridge).

**Why:** People chatting through IRC should be visibly arriving and leaving,
just like DessPlay participants. QUIT and KICK are departures too; ignoring
kicks also leaves stale `/summon` candidates. Notices use the same membership
tracking as `/summon`, so a global QUIT from someone outside the channel cannot
manufacture a departure. NAMES is a snapshot, not a burst of arrivals on every
reconnect. The existing bridge-nick filter avoids duplicate DessPlay presence,
and local-only delivery avoids one synced copy per bridge. A kick targeting
our own bridge ends the session and uses rejection backoff, restoring the
bridge without a tight reconnect loop.

### System messages are derived, not synced

**Rule:** the chat narrator derives system lines locally by diffing successive (state view, peer list) pairs; nothing is sent on the wire; see [design.md](design.md#system-messages).

**Why:** the underlying facts already live in the synced CRDT state or in the server's `PeerList`, so syncing the lines would carry no new information. Because every client diffs the same synced inputs, every client narrates the same lines, consistent without extra wire traffic.

The cost is that a late joiner does not see past events: a transition cannot be reconstructed from a snapshot that holds only the current value. That is acceptable. System lines are a real-time "what's happening now" cue, and the durable answers live elsewhere: the Users pane shows who is present now, and the playlist pane shows the full play history in muted colors. The two exceptions (the player crash, written as a real synced chat message, and the day separators, recomputed from persisted timestamps) exist precisely because those are the cases where a late joiner does need the information.

### Narrator attribution comes from the data

**Rule:** narrator lines attribute changes from map keys and `set_by` fields, never from who wrote the register; new-file lines are un-attributed; watch-preference lines are scoped to the now-playing series; see [design.md](design.md#system-messages).

**Why:** the resolved `StateView` does not record who wrote a register, so attribution has to come from the data. The now-playing writer is not recoverable at all (manual selection takes no seek authority), which is why new-file lines carry no name and EOF-advance is distinguished only by the prior file's watched flag flipping true.

Scoping watch-preference lines to the now-playing series keeps The List's bulk auto-writes for other series out of the chat. The series-preference value carries `set_by: Option<UserId>` (mirroring `ManualState::Away`) for exactly this reason: it is the only way to render "(by Baughn)" when one user rules on another's commitment.

### One narrator line per action

**Rule:** the narrator emits one line per user-meaningful action, not one per register written; server-forced pauses, drift corrections, and position samples never produce lines; see [design.md](design.md#system-messages).

**Why:** the server-forced intent -> Paused on Lost / departure / EOF is already explained by the corresponding lost / left / new-file line, so narrating it as a bare "paused" would be cascade spam. Drift-correction slews and automatic hard seeks never create a `UserSeek`, so they never produce a "skipped to" line; the 1500ms seek debounce coalesces a scrub into one authority write for the same reason.

### Day boundary at 09:00

**Rule:** chat day separators fall on a 09:00 local-time boundary, are computed at render time from persisted timestamps, and are never synced; see [design.md](design.md#system-messages).

**Why:** watch parties straddle midnight, and the small hours still belong to last night's session, so a "biblical" day boundary at 09:00 matches how the group experiences an evening. Making it a view concern rather than a stored event means a late joiner sees the separators too. The boundary is local-time and per-client by design: it is a reading aid, not shared state.

### Changelog is compiled in and day-grouped

**Rule:** `CHANGELOG.md` is compiled into the binary, grouped by calendar day, validated by a test, and shown as a "What's new" modal at startup; the first run skips the modal; see [design.md](design.md#changelog).

**Why:** users never read the commit log, so new features and fixes must be surfaced in-app. The format is Factorio-inspired, without the rigidity. Validating it in the test suite means a malformed entry fails the suite rather than the user; at runtime a bad file degrades to an empty changelog rather than a crash. The first run skips the modal because the user is in the settings screen and the whole history is trivially "unseen".

### What's-new modal swallows other keys

**Rule:** in the "What's new" modal, `Enter` or `Esc` dismisses and every other key is swallowed; see [design.md](design.md#changelog).

**Why:** the modal opens under the user's hands at startup. Both dismiss keys do the same harmless thing, so an accidental Enter costs nothing, and swallowing everything else keeps a startup keystroke from reaching a pane.

### Changelog seen marker lives outside the Settings struct

**Rule:** the `changelog_seen` marker is a bare settings key (`YYYY-MM-DD:count`), not a field of the typed `Settings` struct; see [design.md](design.md#changelog).

**Why:** settings saves round-trip the whole struct from the UI's copy, so a marker field would be clobbered by any unrelated save. The `:count` suffix exists so that entries appended to a day the user already saw are still surfaced later.

### Chat image fetching is https-only and hard-capped (2026-09-07)

**Rule:** inline chat images fetch only `https://` URLs with a known image extension, first URL per message, under wire/time/dimension/allocation caps, with the format sniffed from magic bytes; see [design.md](design.md#inline-chat-images).

**Why:** the URL source includes the IRC channel, which is public and unauthenticated — auto-fetching from it is an SSRF, decompression-bomb, and disk-fill surface. Extension gating keeps a bare link from triggering a speculative request to an arbitrary host; https-only (redirects pinned too) keeps a hostile link from downgrading; the 5 MB / 30 s / 8192 px / 64 MB caps bound what any one link can cost; magic-byte sniffing means a lying extension can't route bytes to the wrong decoder. First-URL-only bounds per-message cost and keeps the row-reservation geometry simple. Failures deliberately show no error chrome — the channel is watch-party chat, and a dead link is not the group's problem to stare at.

### Chat images are per-client fetches, not synced attachments (2026-09-07)

**Rule:** every client detects and fetches image URLs independently; nothing about images enters the CRDT or the wire protocol; see [design.md](design.md#inline-chat-images).

**Why:** inbound IRC lines are already local-only (each member runs their own bridge and receives the same PRIVMSG), so every client sees the URL without help. A synced attachment would need a `ChatMessage` schema migration, would flow into the server's chat archive, and would make the group's sync fan-out carry image bytes — all to deduplicate a capped 5 MB download that a disk cache already amortizes. The `ChatMessage::text` doc states the project idiom: side-channel data rides inline in the text and is decoded at display sites only (the CTCP-action rule); a URL is already exactly that.

### Half-blocks are the default image renderer (2026-09-07)

**Rule:** inline chat images render as truecolor Unicode half-blocks by default (at the terminal's queried font aspect ratio), not via the Kitty/sixel/iTerm2 graphics protocols; `DESSPLAY_IMAGE_PROTOCOL` opts into a real protocol; see [design.md](design.md#inline-chat-images).

**Why:** the first Ghostty run showed the image displaced to the terminal's top-left over the border, plus the whole chat duplicated below. ratatui-image's graphics-protocol renderers place pixels by embedding cursor-moving escapes (`\x1b[s`, `\x1b[u`, `\x1b[{n}C`, `\x1b[{n}B`) *inside cell symbols*, which ratatui's diff-based backend cannot account for; near the screen's bottom edge — where a freshly posted image sits — the downward move scrolls the terminal, and because the UI loop deliberately never issues a clear (the `Terminal::clear` stdin-starvation ban), the scrolled-away frame stays on screen as duplicated content. The emitted escapes look correct in isolation (verified by capturing the byte stream for mid-screen, bordered, color-passed, and bottom-edge cases), so the fault is a live-terminal interaction that can't be fixed without hands-on iteration on the actual emulator. Half-blocks write ordinary `▀` cells with fg/bg colors: they compose with the cell grid, the diff, and the truecolor pass with none of that machinery, and at the real font aspect ratio they look decent for chat-sized images. The env override keeps the graphics path reachable so it can be debugged on a real terminal later. The `image_areas` exclusion in the color pass, the sliced-crop scroll behavior, and the modal suppression all still apply — only the encoder changed.

**Post-mortem (2026-09-07, later the same day):** the "live-terminal interaction" was found. It was not the escapes and not the bottom edge: the terminal had **left/right margin mode (DECLRMM, `CSI ?69h`) left on** by a previously suspended program in the same tab. With that mode on, `CSI s` is DECSLRM ("set left/right margins", which also homes the cursor) rather than "save cursor", so every Kitty image row printed its placeholders at the screen's top-left; the last row written won, and the next frame's row-0 re-send replaced it with the top row. A fresh tab, or any resize (which clears margins), made the same bytes render correctly. The graphics path was never broken on its own; see [The terminal state prologue](#the-terminal-state-prologue). The half-blocks default stays for now because it is the renderer that works everywhere (Ghostty, for one, implements neither sixel nor iTerm2), not because the Kitty path is glitchy.

### The detected graphics protocol is the default (2026-09-07)

**Rule:** inline chat images render with the protocol the startup terminal query reports (Kitty, sixel, or iTerm2), falling back to truecolor half-blocks only when the query reports none or fails; `DESSPLAY_IMAGE_PROTOCOL=halfblocks` opts out; see [design.md](design.md#inline-chat-images). Supersedes [Half-blocks are the default image renderer](#half-blocks-are-the-default-image-renderer).

**Why:** the half-blocks default was a retreat from a glitch that turned out to be inherited terminal state, now neutralised by the [terminal state prologue](#the-terminal-state-prologue); with it in place the Kitty path rendered correctly on Ghostty, and the byte stream had already been verified for the bordered, color-passed, scrolled, and bottom-edge cases. Real pixels are the point of the feature — half-blocks show a 1280-px-wide image at two "pixels" per cell — so the higher-fidelity path should be the one users get without configuration. The query's answer is trusted as-is rather than second-guessed per terminal: a terminal that advertises a protocol it renders poorly is a terminal bug, and the env override exists for exactly that case. The fallback keeps the queried font size, so half-blocks still get the right aspect ratio.

### The terminal state prologue (2026-09-07)

**Rule:** right after entering the alternate screen, the client writes `CSI ?69l` (left/right margin mode off), `CSI r` (scroll region = full screen) and `CSI ?6l` (origin mode off); see [design.md](design.md#inline-chat-images).

**Why:** the client's frames assume absolute cursor addressing (`CSI row;col H` lands where it says) and that `CSI s`/`CSI u` save and restore the cursor. Both are only true in the terminal's default state, and that state is *terminal-wide*, not per screen — Ghostty keeps margins and modes across the `?1049h` switch — so a program that ran earlier in the same terminal and left margin mode on (a suspended TUI with a split-pane renderer is the likely culprit here) silently changes what our bytes mean. The symptom was Kitty inline images rendering at the top-left of the screen while identical bytes rendered correctly in a fresh tab; a DECRQM probe of both tabs found mode 69 as the only difference, and resetting it alone fixed the replay. Establishing the state we depend on, rather than inheriting it, fixes the class: any mode that remaps cursor moves, not just the one that bit. The prologue is not undone at exit — the previous state is unknowable without a query round-trip on stdin (the `Terminal::clear` hazard), and the reset state is every terminal's default.

**Upstream note:** ratatui-image's Kitty renderer uses the ambiguous `CSI s`/`CSI u` pair; the DEC forms `ESC 7`/`ESC 8` are unambiguous and also save/restore the attributes its comment says it wants. Worth reporting; the prologue makes us robust either way.

### Sliced rendering, not crop-resize, for scrolled images (2026-09-07)

**Rule:** an image scrolled partially out of view is drawn with ratatui-image's sliced renderer (the fitted image, cropped by rows), never with `Resize::Crop`; see [design.md](design.md#inline-chat-images).

**Why:** `Resize::Crop` crops the *unscaled source*, so a partially scrolled image would suddenly show native-resolution pixels at a different scale than the fitted view — discovered by a test comparing scrolled against unscrolled cells. Slicing also keeps one cached encode valid across every scroll position; only a pane resize re-encodes. The decode path pre-scales sources to 1280 px for the same reason: re-encodes happen on the render thread, so their input must stay small.

### Inline images hide under modals

**Rule:** while any modal or the work overlay is up, inline images reserve no rows and draw nothing, and the under-modal recent-chat tail never shows them; see [design.md](design.md#inline-chat-images).

**Why:** terminal graphics protocols place pixels outside ratatui's cell z-order, so an image can bleed through whatever is drawn on top of it. Suppressing for the whole modal duration is cruder than occlusion-testing the modal rect but is provably artifact-free on every protocol. The recent-chat tail additionally renders at a second rect, which would thrash the per-size encode cache for a dim context strip.

### The image-protocol query gates the input thread (2026-09-07)

**Rule:** terminal image-protocol detection runs on the UI thread after entering the alternate screen, and the input thread is only spawned afterwards, via a callback that fires on every setup path; see [design.md](design.md#inline-chat-images).

**Why:** the query writes an escape sequence and reads the terminal's reply from stdin. The input thread's blocking `event::read` would consume that reply — the same stdin-ownership hazard behind the long-standing `Terminal::clear` ban in the UI loop. A terminal that fails or never answers the query falls back to half-blocks with an assumed font size, so detection can never cost more than a startup delay; the callback fires from a drop guard so a failed terminal setup still releases the caller.

## Client Roles

### Primary seeder is colocated with the rendezvous server

**Rule:** The primary seeder runs next to the rendezvous server and connects over loopback; see [design.md](design.md#seeder-behavior).

**Why:** Serving a file then costs one trip over the NAS uplink per recipient. The relay-only transfer design (no client-to-client connections) depends on this arrangement being the common case; without colocation every relayed byte would cross the seeder's uplink and the server's uplink both.

### Seeders persist a hash cache but no settings

**Rule:** A seeder is configured purely by flags/env and persists no settings, but it does persist the hash cache and cache bookkeeping in a database, and it rescans its media roots once a day rather than once a minute; see [design.md](design.md#seeder-behavior).

**Why:** A seeder may hold terabytes, so re-hashing its store on every startup is a nonstarter; the hash-addressed cache lets it re-discover prior downloads by hash on restart without a media-root filename scan. Its store is big and stable, so a minute-cadence rescan would be wasted work. Settings are not persisted because seeders run as systemd services from a NixOS config and never show the settings screen.

## Presence

### Graceful quit skips Lost but keeps the commitment

**Rule:** `/quit` / Ctrl-C moves the user straight to Departed, keeps them listed like a timed-out peer, still gates if they are committed to the now-playing series, forces playback intent to Paused if playback was running, and hands seek authority back to the server at once; see [design.md](design.md#presence).

**Why:** Skipping the Lost stage is the *only* thing a clean quit buys over a silent disconnect. It does not waive a commitment: per User States, the group waits for a committed user even when absent, "Lost, Departed, or quit". Playback pauses because leaving mid-episode should not be silent. The server reclaims seek authority immediately because a clean quit is final, unlike a Lost that may recover.

### Known-offline users are one list and valid skip targets

**Rule:** The Users pane renders every known-offline user (within 30 days) as one dim italic list, with "last seen" ages, whether they left minutes ago or have not shown up today; `n` and `/skip <name>` work on any of them; see [design.md](design.md#presence).

**Why:** The server's in-memory registry only spans its own process lifetime, so a user who has not connected since the last server restart is otherwise invisible. Both kinds of absent user are equally valid `n` / `/skip <name>` targets, and the point of the list is to let the group rule on someone's commitment without waiting for them to reconnect. Unifying them avoids two lists that mean the same thing.

### Known-offline users gate for seven days

**Rule:** Clients synthesize a Departed interactive peer entry for each known-offline user seen within the last seven days before deriving playback gating, so a committed absent user keeps blocking across server restarts; past seven days the synthesis ages out; see [design.md](design.md#presence).

**Why:** Playback gating quantifies over peer entries, and the server's registry spans only its own process lifetime, so a server restart would silently waive every absent user's commitment. The seven-day horizon matches the commitment's own "wait for me even if I've been gone a week" phrasing. It also bounds how long a stale username can keep blocking — for example after a naming-convention change, since commitments are keyed by username. Identity aliasing was deliberately rejected as a fuzzy heuristic in the gating path; the bounded horizon plus the explicit dismissals (`/skip <name>`, Away, per-file `/ack`) cover the stale case instead.

## The List (Series Tracker)

### AniDB linking is enrichment only

**Rule:** A List entry's `anidb_series_id` provides episode metadata, franchise grouping, and the search modal, and is never a prerequisite for commitment or gating; see [design.md](design.md#the-list-series-tracker).

**Why:** A real number of series the group watches — obscure OVAs, doujin work, non-Japanese content, very new simulcasts — simply have no AniDB entry, and the design must not depend on them getting one. Commitment therefore keys on the entry's `ListEntryId`, never its AniDB link.

### next_ep is free text

**Rule:** `NextEpState.next_ep` is `Option<String>`, not a number; see [design.md](design.md#schema).

**Why:** Real spreadsheet entries include season prefixes, OVA names, and guesses ("12", "S3-05", "Sisters", "movie 5?"). A numeric field could not represent the data the group actually keeps.

### The List is never pruned

**Rule:** List entries are never compacted or removed; see [design.md](design.md#schema).

**Why:** It is a few hundred rows of text, and the history is the point.

### Unlinked entries carry their own identity data

**Rule:** Unlinked List entries resolve files through `local_aliases` and `manual_files`; a linked entry's AniDB series id is authoritative and skips both. The resolution order is deliberately stricter than the franchise-browsing and known-series heuristics; see [design.md](design.md#series-identity).

**Why:** Two mechanisms already turn a file into "a series," and neither is a safe foundation for group commitment (whether the group waits for someone across absence):

- AniDB's relations graph, for files with a series id — structural (sequel/prequel/etc.) and stable, but only exists for series AniDB knows.
- The per-file derived name (the AniDB-miss fallback's `series_hint`, or the bare filename otherwise), used for franchise-browsing's fallback grouping and personal known-series detection. This name is not stable enough to hang commitment on: it is computed per file, from that file's own directory context, and the group does not reliably keep every episode of an untracked show in one dedicated directory. Two files of the same show, one hinted from `Anime/ShowName/` and one sitting loose elsewhere, derive different names — silently splitting one show into two "series" for the one question that most needs a single stable answer.

So entries carry confirmed aliases (seeded from the first file's derived name, grown by hand) and explicit file hashes for outliers whose name parses into no alias at all. The browsing heuristics stay soft and unchanged because a mis-grouped franchise row is a browsing annoyance, but a mis-resolved List entry silently un-commits someone from a show they are actively watching.

### Commitment is per franchise, not per season (2026-08-28)

**Rule:** Resolution step 1 matches a file to any List entry linked anywhere in its structural-relations franchise; with several linked entries the canonical one answers and a user's commitment is the fold Watching > NotWatching > Maybe over all of them; see [design.md](design.md#series-identity).

**Why:** Proposal [2026-08-28-franchise-list](proposals/2026-08-28-franchise-list.md). `/watch` on season three should commit to the show, and a new season should never mint a second entry. Legacy per-season duplicates already existed, so the canonical-entry rule (human-created over auto-created, then deepest along the prequel chain, then lowest id) and the preference fold let them coexist without a migration. The one-hop check from each linked season's own relations row covers a brand-new season whose relations row has not landed yet.

### Bumping next_ep is certain; resolving it to a file is not

**Rule:** The server bumps `next_ep` from the just-finished file's episode number (AniDB for linked entries, filename-parsed for unlinked ones), but jumping to the next episode of an unlinked entry goes through the Episode Browser's candidate-ranked disambiguation view rather than queueing a guess; see [design.md](design.md#advancing-next_ep).

**Why:** Two distinct problems hide under "auto-advance," with very different certainty. Bumping the counter from the finished file has no real ambiguity: it is a fact about a file already confirmed watched, not a guess about one that has not aired yet. Finding *which* library file is episode `next_ep + 1` is the genuinely uncertain step for an unlinked series — there is no AniDB episode identity to match against, only heuristics. Rather than guess silently, the existing multi-file disambiguation UI ("several files claim the same episode number ... expand into a lightweight tree") is generalized from "several files, one confirmed identity" to "several candidate files, ranked by score, no confirmed identity." This is deliberately not a new kind of synced Playlist entry (no `Map<Ed2kHash, ...>` schema change); it lives entirely in the Series/List pane and the episode browser, exactly like choosing which copy of a linked episode to play today.

### One List row per franchise (2026-08-28)

**Rule:** Entries linked into the same relations component collapse into one List row showing the canonical entry, the union of commitment initials, and `available` if any member is; see [design.md](design.md#ui-integration).

**Why:** Proposal [2026-08-28-franchise-list](proposals/2026-08-28-franchise-list.md): commitment, recency, and progress are franchise-level facts, and per-season rows duplicated them. The recency sort likewise takes the newest watch of *any season in the franchise*, entry or not, so "the latest episode is in season three" still floats the row.

### Dim List rows are never reordered (2026-08-28)

**Rule:** Rows with nothing to watch render dim in either sort but keep their sort position; see [design.md](design.md#ui-integration).

**Why:** Proposal [2026-08-28-franchise-list](proposals/2026-08-28-franchise-list.md): a predictable order beats a partition that shuffles as files come and go.

### Watchable ignores who currently advertises a copy (2026-08-29)

**Rule:** A List row is watchable when `available` is set or any known library file for it is unwatched; whether some peer currently advertises a copy is not a condition; see [design.md](design.md#ui-integration).

**Why:** User decision 2026-08-29, matching the episode browser's season rows: the library index is the durable record of what exists, and a show nobody happens to be seeding tonight is not "nothing to watch".

### Curated short titles replace the official name; human edits win (2026-08-18)

**Rule:** A linked entry whose relations row carries a curated short title renders and alphabetizes under it instead of the official name, but only while the entry's name still equals the official title; a human-typed or edited name always wins; see [design.md](design.md#ui-integration).

**Why:** User decision 2026-08-17: save the space — "GochiUsa", not "Gochuumon wa Usagi Desu ka??"; the full name still lives in the edit modal and the episode browser. User decision 2026-08-18: a name a human typed or edited always wins, which is also the fix-it path for a bad curated pick.

### Short titles are AI-curated with a client-provisioned token

**Rule:** The server asks an Anthropic model for each series' short title, caches the answer forever, settles repeated declines as "no short name", and gets its API token pushed from one client's settings (`SetAnthropicToken`, protocol v12); see [design.md](design.md#ui-integration).

**Why:** The titles dump's kind-3 rows are lowercase search tags ("gochiusa s2", "s;g", "HnNKn") and only a quarter of series have one, so they cannot be read raw. The answer is trusted as returned because the human-name precedence above is the backstop; answers for series not in the batch are dropped as a sanity guard. Settling after a few declines means no series is billed indefinitely. The token is client-provisioned rather than server-configured so the settings screen is the whole lifecycle interface for rotating or removing the server-side credential, reusing the `anthropic_token` the commentary engine already stores.

## Real QUIC/UDP tests are opt-in (2026-09-15)

The real-socket QUIC, localhost integration, and DSCP wire tests retain their
OS interfaces and carry `#[ignore = "Requires sandbox escalation"]`. The
ordinary gate runs in restricted sandboxes; transport, socket configuration,
and QUIC dependency changes explicitly run this socket coverage with suitable
permissions, as documented in testing-strategy.md.

The transport is stable enough that running these tests on every unrelated
change adds little value. In-memory substitutes would lose the important OS
integration checks. Explicit ignore annotations replace the separate sandbox
profile and work with both Cargo's built-in runner and nextest. Simulated
network and pure protocol tests remain in the default gate.

## TUI Layout

### Layout watches exclude unrelated directory trees (2026-09-15)

Recursively watching the override directory's parent made Linux live reload
depend on access to every sibling. The two tests with overrides directly under
`/tmp` failed because notify's inotify backend stopped registration at an
unreadable Nix directory. macOS's different watcher backend hid the problem.
This was a production watch-scope bug, independent of Codex's socket restrictions.

Only the override tree now gets recursive watches. Non-recursive watches follow
its ancestor chain, and registration is rebuilt before each debounced load.
This retains atomic saves and missing/replaced-directory recovery, including
replacement of an ancestor, without crawling siblings or consuming watches for
their descendants. Registering before reading includes changes made while
watches were being replaced; subsequent events invalidate that compilation.
Moving tests into private parents alone would have hidden the production bug.
An unreadable-sibling regression was confirmed failing before the fix.

### Runtime templates retain synchronous controllers (2026-09-07)

Layout authoring must reach semantic widget interiors, rather than only
moving opaque panes. The migration starts with shared form composition and
label/value/annotation rows plus the diagnostic log, preserving their
existing controller-owned editors and selections. Remaining surfaces are
explicitly tracked before claiming whole-display customization.

XML and CSS compile into immutable, transferable bundles. quick-xml handles
markup; cssparser supplies CSS tokens and locations; Taffy allocates boxes.
Browser-style error recovery is disabled at the authoring boundary: an
incomplete save or unsupported feature retains the working bundle. Embedded
assets use the same compiler. Width-first measurement prevents wrapped
height from feeding back into terminal width, and clipping paints original
edges without allocating buffers proportional to authored overflow.

The shell constructs the renderer after entering the UI thread. Putting its
Taffy tree in `Ui` would compromise the existing pre-thread controller
construction and Send boundary. A separate bounded reload lane coalesces
editor saves, retries full-channel delivery, and rejects generations
invalidated by newer filesystem events. It sleeps while idle. The F12
recovery surface uses embedded definitions so customization cannot remove
the recovery controls. Native GUI rendering and scripting remain outside
this change.

### Shared chat fragments preserve source identities (2026-09-07)

The message prefix and body now use the same flow service as other rich text.
A distinct prefix policy preserves the existing first-line formatting without
requiring pre-padded strings; separate fragments allow body source offsets to
remain correct when wrapping drops boundary whitespace. Wide first glyphs
move to the continuation row rather than disappearing behind a long prefix.
Selection and spoiler hit regions derive from the painted fragments.

Live chat lays out only the tail window or the retained scroll anchor. Scene
cache keys distinguish fixed-height and natural-height requests, preventing
a clipped measurement from becoming an intrinsic row height. Cosmetic spoiler
marks are source-indexed paint data, and ASCII scrambling with unchanged word
boundaries refreshes cached spans without remeasuring. Arbitrary Unicode or
style changes still take the normal measured path.

### Series rows and filter captions share measured geometry (2026-09-07)

Recent and All Series, List headings, and List entries now supply semantic
fields without fixed-column padding. The collection renderer retains existing
centering and controller selection, including duplicate entries under distinct
groups. A before-attached flow paints the caption and editor cursor on the
frame edge, avoiding a widget-owned coordinate calculation or a second text
measurement path for rich captions. Existing default List snapshots remain
unchanged; XML-only row reordering retains edit targets through reload.

### Modal slots follow template stacking order (2026-09-08)

Browser/search and small-dialog migrations use slots painted within the scene
traversal. Painting all chrome first and all primitive contents afterward would
let later body painting erase an earlier-declared nested popup. The renderer
records subtree boundaries and paints nested overlays after their enclosing
content, then proceeds to the next sibling overlay.

Full dialogs use a separate bounded modal placement policy. Reusing the
recovery panel's one-cell safety inset shifted browser frames on short
terminals and clipped the bottom edge, caught by the existing season snapshot.
Modal percentages floor to cells and minimum sizes yield to the viewport,
preserving the existing browser dimensions and centering. The property gate
compares arbitrary small sizes and translated origins against that contract.

### Repeated items have their own binding and identity scope (2026-09-08)

A repeat contains one item root and consumes a typed list. Validating that root
against the enclosing form would confuse fields such as tab labels with form
titles; item-scoped validation catches those errors before installation. Rust
provides stable keys, and arranged IDs include their list/key scope while CSS
selectors retain local template IDs. Reordering data therefore retains its
selection identity without imposing order-derived IDs on template authors.

Settings tabs and notes now supply semantic fields instead of preassembled
lines. Small repeats arrange eagerly under explicit item/node limits. Large
collections retain their existing virtualized renderer while authored virtual
repetition is completed. Paint-only cache refresh remains disabled for list
containers until it can resolve nested rich spans by their complete item scope;
unchanged lists still hit the exact scene cache.

### Viewport boxes preserve root CSS and document anchors (2026-09-08)

The renderer gives each component a real single-cell containing grid instead
of overwriting its root width and height. The previous forced dimensions
ignored root margins when wrapping and omitted them from row extents; adjacent
rows could consume margin space, and explicit root dimensions did nothing.
Regression tests confirmed those failures before the shared allocation fix.
Collection, chat, and document rows now consume the same complete measured
extent, and hit targets still exclude margins.

Changelog entries use semantic prefix/body fields and the shared document
scroller. Its controller stores an item/source anchor and pending row deltas.
An absolute wrapped-row counter would become stale after resizing and could
prevent scrolling back to the actual beginning. Only the visible window and
rows traversed by a navigation request need layout; exact scene-cache hits
avoid remeasurement on redraw. Status and keybinding fields use the same
text/repetition pipeline while their existing keymaps remain in controllers.

### Explicit dark theme on true-color terminals

**Rule:** A true-color terminal gets an explicit app-wide dark theme with RGB semantic foregrounds; dim text is an explicit muted RGB, never SGR 2; limited-color terminals keep their own theme and the ten-color palette; see [design.md](design.md#ui-principles).

**Why:** Painting the whole alternate-screen buffer with a known background and mapped foregrounds makes text contrast deterministic instead of depending on the user's terminal theme. Dim text is materialized as RGB because the treatment of SGR 2 alongside explicit RGB colors varies between terminal emulators. The color capability is injected into the synchronous `Ui` rather than read from process-global terminal state so that tests do not depend on the terminal they happen to run in.

### Visible progress for every long-running operation

**Rule:** Anything that can take more than a moment shows visible progress while it runs, and the status bar shows the server link whenever the client is not connected; see [design.md](design.md#ui-principles).

**Why:** A user who sees nothing happen assumes nothing is happening, and retries. The progress overlay for playlist-add hashing is visually modal but captures no input so chat keeps working underneath. The server link is shown for the same reason: stale gating text ("⏸ paused") while the client silently fails to connect reads as a hang, and a dead handshake can take the full per-address timeout ladder before it gives up.

### Progress bar on its own terminal-wide row

**Rule:** The progress bar + time, the health metrics, and the suggestion slot share one terminal-wide row above the status bar, reserved before the column split; see [design.md](design.md#ui-principles).

**Why:** The bottom status bar carries the variable-width "waiting on ..." blocker text, which would shove the bar sideways as blockers come and go; a row of its own gives it the same placement in every subtitle mode. Reserving the row before the column split keeps the playlist's bottom border level with the chat input's.

### Health line exposes sync starvation on a live connection

**Rule:** The right end of the bottom row shows ▲/▼ throughput (QUIC plus torrent), rtt from datagram probes, and seconds since anything arrived from the server; see [design.md](design.md#connection-health-line).

**Why:** A saturated uplink can let BitTorrent drown CRDT sync while the QUIC connection stays nominally "connected"; this row is what makes that visible. The torrent engine's speeds are folded into ▲/▼ so the culprit of a saturated uplink shows even though that traffic never crosses the server connection. The rtt comes from the time-sync probes rather than QUIC's estimate because the probes are datagrams and reflect real path latency (bufferbloat shows up as seconds). The server broadcasts a `StateHash` every 30s unconditionally, which makes the sync-age field a zero-false-positive stalled-sync detector.

### Sync age reads sync ok until remarkable

**Rule:** The sync-age field renders as a static `sync ok` until 5s of silence during group playback, or the 40s warning threshold when alone or idle; display only; see [design.md](design.md#connection-health-line).

**Why:** A counting number draws the eye, and what counts as remarkable depends on how chatty the wire should be. During group playback peers' position datagrams arrive continuously, so 5s of silence is already notable. Alone or idle the only incoming traffic is two interleaved 30s heartbeats, so the age legitimately sawtooths toward ~30s and is only worth showing past the 40s threshold, where it colors anyway.

### Health level hysteresis

**Rule:** The displayed health level shows trouble immediately but must hold calm ~5s, stepping down through intermediate levels, before relaxing; see [design.md](design.md#connection-health-line).

**Why:** Without the filter a single quiet sample flickers the row from red back to dim.

### Suggestion slot hold, truncation, and precedence

**Rule:** A cleared advisor condition holds the slot ~30s but a disconnect clears it at once; under width pressure the metrics keep full width, the bar truncates next, and the suggestion is dropped rather than shown as an ellipsis; the slot's reservation is text-width-capped; precedence is warning/critical suggestion > live marquee > info suggestion > blank; see [design.md](design.md#connection-health-line).

**Why:** The 30s hold guards against threshold flicker. On a full disconnect the `link:` notice supersedes the suggestion, and a condition that persists across the reconnect simply re-emits. The health metrics keep their width because they are the row's reason to exist. A lone ellipsis conveys nothing, so a suggestion that does not fit is dropped. The text-width cap (occupant text plus the 2-space margins, the marquee included) means the bar never shrinks further than the middle actually needs. Commentary yields to health warnings because a health warning is the row's job.

### Commentary model and request shape

**Rule:** Commentary calls the project's Anthropic model with adaptive thinking at low effort, hardcoded, jittered ±15 s per tick; see [design.md](design.md#ai-commentary-the-marquee).

**Why:** The feature is just for fun and explicitly a single-user gimmick. Adaptive thinking is the forward-compatible request shape; the deprecated fixed thinking-token budget is never used. The jitter keeps the comments from feeling metronomic.

### Persistent commentator with a 5% re-roll

**Rule:** The commentator persists across ticks and API failures, is re-rolled with 5% probability per tick, is not reset on a series change, and keeps a character card pinned to its home series; see [design.md](design.md#ai-commentary-the-marquee).

**Why:** A quietly changing persona is funnier than a fresh voice every time. The voice deliberately follows the group to the next show — Hinamori Amu commenting on Grave of the Fireflies is an accepted (welcomed) outcome — until the dice or a client restart retire it. Pinning the card to the home series is what lets a carried-over voice know it is watching someone else's show.

### Commentary thread structure

**Rule:** Each commentator is a multi-turn thread: subtitle turns are cursor-delimited and speaker-attributed, episode identity is keyed by file, a re-roll cuts the thread and seeds the new one with prior comment text only, a ~10-turn cap force-re-rolls, sent history is append-only, and a per-thread screenshot-byte budget sends turns frameless once exhausted; see [design.md](design.md#ai-commentary-the-marquee).

**Why:** Speaker attribution exists because a model that cannot watch the video needs the dialogue attributed; the ASS Name field is the same one the separate subtitle pane colors by. The cursor is the advisor ring's per-line sequence numbers so consecutive turns never overlap and a failed call does not advance it.

Episode identity includes the now-playing file because AniDB-unknown files all share one hint-derived series name and no episode number; without the file in the key an unlinked series' episode changes would never re-header or reset the comment seed.

Seeding a fresh commentator with the text of the current episode's earlier comments (never the images or subtitles behind them) lets the voice change without the conversation restarting from nothing.

The 5% re-roll keeps threads young in expectation, but its tail is geometric, so the hard cap at ~10 turns backs it up, going through the same fresh-thread path the dice take.

Sent history is append-only because the prompt cache matches on a byte-stable prefix. An earlier design instead stripped screenshots from turns older than the last two; that rewrote the cached prefix every tick and silently re-billed the whole thread at full price whenever frames flowed. With history frozen, the turn cap is what bounds the request body, and the screenshot-byte budget (two worst-case frames' worth) keeps accumulated frames from outgrowing the API's request-size cap.

### Commentary prompt caching by interval preset

**Rule:** The 2 min and 4 min presets set an ephemeral `cache_control` breakpoint on the final text block; the 10 min preset sets none; see [design.md](design.md#ai-commentary-the-marquee).

**Why:** The Anthropic prompt cache's ephemeral TTL is 5 minutes. The 4 min preset exists precisely to duck under that TTL with jitter included (which is also why the settings ladder is 4:00 rather than 5:00). At 10 min the cache would be cold anyway, so the write surcharge is skipped.

### Marquee distribution and replay rules

**Rule:** Comments go through a generic synced marquee register; a pass is keyed by LWW stamp; a stamp from before the session's first snapshot is adopted as already-played; the text enters fully off-screen right; the UI ticks at ~100ms only while a pass animates; see [design.md](design.md#ai-commentary-the-marquee).

**Why:** The register is deliberately generic so marquee sources beyond commentary can use it later. It persists in synced state until compaction, so without the pre-startup rule a freshly started client would replay last night's final comment on launch. The off-screen entry delay gives people time to notice motion and glance down before the sentence starts leaving. The faster tick costs nothing when idle because a tick only repaints when something moved.

### Commentary failures are silent and the log is loud

**Rule:** Every commentary failure is a log line and a skipped tick, never user-visible; the feature logs its enablement, requests, commentator, token usage, and replies at info; see [design.md](design.md#ai-commentary-the-marquee).

**Why:** A gimmick that speaks once every few minutes is otherwise indistinguishable from a broken token. Logging per-call cache read/write usage means "is the cache hitting?" is one grep away.

### Wheel scrolls only the focused pane

**Rule:** The mouse wheel scrolls a pane only when it is already focused, except the non-focusable subtitle pane, which the wheel scrolls whenever the pointer is over it; subtitle scroll-back is mouse-only; see [design.md](design.md#tui-layout).

**Why:** Touchpads emit wheel events by accident, so a graze must neither scroll invisibly nor steal focus. Subtitle scroll-back has no keyboard path because keyboard users scroll subtitles in Intermixed mode, where they share the chat log.

### Mouse actions have key equivalents except resize and selection

**Rule:** Every mouse action has a keyboard equivalent, except pane resizing and chat text selection; see [design.md](design.md#tui-layout).

**Why:** Resizing is rare and not something keyboard speed matters for. Chat text selection is mouse-native and uncommon enough to need no keyboard path.

### Drag to reorder previews and commits once

**Rule:** Dragging a playlist entry or a media root shows the move
locally, within a viewport frozen at the press, and commits one
identity-anchored move on release. A drag reaches only the visible rows.
The settings modal is the only modal that takes the mouse; see
[design.md](design.md#tui-layout).

**Why:** The playlist is synced state. Committing at every row crossed
would broadcast a burst of CRDT ops for one gesture, and the playlist
would jump at peers mid-drag. The move is recorded as "entry after
entry" by identity, the shape `J`/`K` already sends
(`MovePlaylistAfter`), because snapshots replace the rows about ten
times a second during playback, and peers may add or remove entries
mid-drag. An index-keyed grab would retarget onto a neighbour (the bug
class behind the series pane's `ListAnchor`). For the same reason, a
press that stays on its row never moves anything, even if the rows
shift underneath it.

The viewport freezes because focused lists center on the cursor. A
press moves the cursor, so without freezing the next frame would recenter
and slide the pressed row out from under the pointer. Hit-testing
against press-time geometry while rendering the press-time center keeps
the screen and the pointer mapping in agreement. Auto-scroll was
rejected because terminals report drags only on motion, so there is no
timer to scroll by, and `J`/`K` already covers long moves.

The settings modal takes the mouse because media roots are the only
other reorderable list. Click-to-select comes with it, under the
existing rule that a click never activates a row. Other modals still
ignore the mouse; nothing in them needs it.

### Chat selection copies on release with no copy key

**Rule:** Drag-selecting chat text copies to the clipboard on release (CLIPBOARD and PRIMARY on X11), a hidden spoiler copies as its scramble, and cross-message selections snap to whole lines in irccloud log format; see [design.md](design.md#tui-layout).

**Why:** Once mouse capture is on, the terminal's own selection needs Shift and knows nothing of panes, so the app provides its own. There is no copy key because the terminal owns Cmd-C and Ctrl-C stays Quit. PRIMARY is written for the terminal-user reflex of middle-click / Shift-Insert. Copying the scramble rather than the hidden text is WYSIWYG and cannot leak a spoiler. Day separators are render furniture, so they are skipped. Widening a partial selection to its whole line before extending follows the gdocs convention.


### Live diagnostics and indexing reasons (2026-09-05)

**Rule:** F11 shows a bounded live log over the upper two-thirds of the screen,
with independent, session-only DessPlay and dependency logging levels. Default
logs explain every cache-backed hashing decision with old/new metadata; see
[design.md](design.md#diagnostic-logs).

**Why:** Dagger reported a recurring indexing notice without debug logging
enabled. Counts and paths could show the repeated work but not why the cache
was rejected. The decision point must record its evidence at info level, and
hash failures must be visible before the next attempt. This adds diagnostics;
it does not infer or fix the cause of that report.

An in-app tail lets a player inspect current activity and enable detail without
restarting. Keeping the bottom chat lines visible preserves party context.
The same formatted stream feeds the daily file and memory, so changing the
level captures the evidence for later inspection too. Bounds on retained lines,
bytes, and individual displayed events keep trace logging from growing memory
without limit; disk files retain complete events under the existing rotation.
A stable line identity prevents scrollback moving under the reader during
appends or eviction.

Separate workspace and dependency scopes let a player enable application trace
without enabling every networking dependency. Overrides last only for the
session (the user's preference), and Startup restores each scope's original
filter independently, preserving target-specific RUST_LOG settings. Polling the
buffer revision on the existing UI idle tick avoids a log-event channel flooding
the UI queue or tracing its own delivery indefinitely.


## Network Protocol

### Compaction hour and server placement

**Rule:** compaction runs daily at 12:00 UTC by default, and the rendezvous server is colocated with the primary seeder on the NAS; see [design.md](design.md#rendezvous-server).

**Why:** 12:00 UTC was chosen to be far from watch-party hours, so a compaction (which bumps the epoch and makes every client re-adopt a snapshot) never lands mid-episode. The NAS placement matters because the transfer design is relay-only: with the seeder on loopback next to the server, serving a file costs one trip over the NAS uplink per recipient, and the relay-only design depends on that arrangement being the common case (see Client Roles, Seeder Behavior).

### UI animation on a local monotonic clock

**Rule:** all state timestamps use the shared clock, but the TUI's animators run on a local monotonic clock and use shared/wall time only for display and message identity; see [design.md](design.md#time-synchronization).

**Why:** both the wall clock and the shared clock can step backward — NTP corrections on the wall clock, and a later time-sync round shrinking the computed offset on the shared clock. An animator driven by either would stall or jump on a step. A monotonic clock cannot step, so animation timing is immune; the shared clock is still the right identity for messages and the right value to display (see ui-architecture.md).

### LwwCell instead of crdts::MVReg

**Rule:** every replicated register is `LwwCell<V>`, DessPlay's own max-merge last-writer-wins register; see [design.md](design.md#state-sync-protocol).

**Why:** `crdts::MVReg` proved non-convergent when nested inside `crdts::Map` — replicas could end up holding different values after the same set of merges. A max-merge LWW register over the shared clock converges by construction. Details of the `Lww<V>` design and the failure are in sync-state.md.

## File Management

### Scan hashing yields to transfers

**Rule:** while transfer traffic is active, library-scan hashing defers and resumes ~10s after the traffic goes quiet; the stat-only walk continues; a walked file whose name matches an unmet playlist entry is hashed immediately regardless; see [design.md](design.md#media-library-scanning).

**Why:** scan hashing is bulk disk work with no deadline, while transfers are latency-sensitive — a source that serves nothing for 30s is snubbed by the requester (network-design.md). Letting a rescan compete for disk bandwidth mid-transfer risked exactly that snub. The exemption exists because without it an active download would defer the discovery of the very local file that makes the download redundant ("a local copy trumps the download"): the file would sit on disk, unhashed, while the client kept fetching it from a peer.

### Vanished roots and the removed-root grace period

**Rule:** a root none of whose indexed files exist is marked vanished and its hashes are retained indefinitely; a root with at least one surviving file has its missing rows pruned at once; a removed root's index rows survive for a seven-day grace period; see [design.md](design.md#media-library-scanning).

**Why:** the all-files-missing case is taken as removable storage being disconnected (an unmounted NAS or external drive), not as a mass deletion. Pruning those rows would force a full re-hash of a possibly multi-terabyte store on remount, so the hashes are kept and the root simply hidden until any recorded file returns. When at least one file still exists the root is demonstrably online, so missing siblings are genuine moves or deletions. The seven-day grace on an explicitly removed root exists only so that re-adding the identical path is cheap (no re-hash); it is bounded so a root the user really did abandon does not keep its index rows forever. A configured-but-vanished root never expires because the user has not removed it — it is merely unplugged.

### Library-wide lookup requests and AniDB load

**Rule:** every indexed hash lacking metadata is inserted into the `lookup_requests` GSet with its mtime and a directory-derived `series_hint`; see [design.md](design.md#media-library-scanning).

**Why:** the scan has each file's path and mtime in hand anyway (the mtime keys `hash_cache`; the path yields the directory hint), so carrying both costs nothing and gives the server the inputs it needs for the AniDB-miss fallback name and for the age-anchored re-validation ladder. Feeding the whole library, not just the playlist, into the lookup set is what lets the franchise browser span the group's collective collection. The AniDB budget stays bounded even when several clients index overlapping collections because the server de-duplicates per hash and the `anidb_queue` table records what has already been checked across clients.

### Resolution reads the index instead of hashing candidates

**Rule:** file matching looks the entry's hash up in the library index, falls back to a single exact-basename search, and otherwise marks the entry Missing and waits for the hash to enter the index; it never walks the disk hashing candidates; see [design.md](design.md#file-matching).

**Why:** the scanner builds and maintains the index ahead of demand, so resolution can assume it already exists; a copy the index has not yet absorbed is picked up by the wanted-set check when the scan reaches it. The basename search exists only to close the gap between "the file just appeared" and "the next scan pass" (a copy dropped in moments ago). It is an optimization: if it misses, nothing is lost but a minute. Hashing every candidate on demand would turn each playlist add into a disk walk, and was rejected.

### Cache reconciliation and the orphan sweep

**Rule:** at startup the file actor reconciles `cache_entries` against disk — pruning rows whose file is gone or mis-sized, re-registering survivors as servable — and deletes hash-named cache files with no row that are older than a week by mtime; see [design.md](design.md#download-cache-and-retention).

**Why:** the `cache_entries` table is an index over the cache, not an authority: a user may delete, move, or truncate files behind the app's back, so the filesystem has to be the source of truth. Pruning a stale row makes the playlist entry honestly re-resolve to Missing and re-download instead of advertising a copy that does not exist. The orphan sweep exists because eviction only iterates `cache_entries`: a hash-named file with no row is invisible to it and would leak forever. An orphan is either a completed download whose bookkeeping was lost (a DB reset leaves the files but not the rows) or an abandoned peer-download partial (`download_path` is the final `<cache>/<hash>` path, so an interrupted download leaves one). The one-week threshold matches the "in-flight downloads don't survive restarts" contract while leaving anything recent alone, since it may still be in flight or wanted.

### Serve-time answers: nothing versus CannotServe

**Rule:** a solicitation for a file the session has not registered is recovered from the library index; if nothing on disk backs the advert the holder answers nothing and retracts its Ready; `CannotServe` is sent only for a definitive identity mismatch; see [design.md](design.md#download-cache-and-retention).

**Why:** after a restart, Ready is durable synced state while the servable set is rebuilt lazily, so an unregistered-but-held file is the normal post-restart condition, not a loss — a live, visible index row bearing the hash is a genuine copy and can be adopted on the spot. The requester treats `CannotServe` as a denial that lasts as long as the advert that earned it stands (it drops the source rather than re-asking forever, network-design.md). Answering a transient "not right now" with it would therefore permanently remove a source that was merely slow to register; answering nothing lets the requester's ordinary source refresh drop and later re-add the holder.

### Eviction rules: unreferenced files and the adoption gate (2026-08-21)

**Rule:** a cached file is evictable once watched or once no playlist entry references it; eviction passes run at startup and on EOF-advance, never touch now-playing or queued unwatched entries, and do not run until a synced state has been adopted this session; see [design.md](design.md#download-cache-and-retention).

**Why:** the "unreferenced" clause exists because an abandoned download must not pin cache space just because nobody happened to watch it. The adoption gate came out of the 2026-08-21 review: before a synced state is adopted, the replica is transiently empty — a first run, or the window after `--reset-sync`/`/resync` before the connect handshake — and an eviction pass planned from that view protects nothing. It deleted cached media the real playlist still referenced, including the now-playing file. Gating on the sync actor's `adopted` watch makes the pass wait for a view that actually describes the playlist.

### Archive layout has no Season level

**Rule:** archiving produces `[Series name]/[Original filename]` (or just `[Original filename]` with the subdirectory setting off), with no `Season #` directory; see [design.md](design.md#download-cache-and-retention).

**Why:** AniDB models each season as its own anime (a franchise member), so a single series name already denotes one season's folder. A separate season level would either duplicate that or invent a numbering the metadata does not carry.

### Auto-archive trigger and ordering

**Rule:** auto-archive fires on the personal 85% watch record, not the group watched flag; a file watched off a partial archives at download completion; it always precedes the EOF-advance eviction pass; the policy is owned by the file actor; see [design.md](design.md#download-cache-and-retention).

**Why:** the group's watched flag (the `w` key, EOF-advance) is the group's history, not this user's viewing — a user who skipped an episode should not have it archived into their library. The partial case is covered because a file watched off a still-downloading partial only becomes a cached download when the download completes, so that is the first moment it can be archived. Because the personal record fires at 85%, auto-archive necessarily precedes the EOF-advance eviction pass, which is what makes `cache_retention: 0` and auto-archive compose (watched files are moved, never deleted). The file actor owns both the subdirectory layout and the auto trigger, pushed on settings save, so the manual `A` path and the automatic path can never disagree about the destination.

### Archiving an open file without a reload

**Rule:** a same-filesystem archive renames inline; a cross-device archive copies in a background task with the cache copy servable until the copy lands; the session updates its resolution and loaded path without reloading the player; see [design.md](design.md#download-cache-and-retention).

**Why:** an archive can move a file the player currently has open. A multi-gigabyte cross-device copy must not stall serving mid-session, hence the background task and the still-servable cache copy. The player is not reloaded because a reload at the 85% mark would be a visible hiccup for the viewer; the bookkeeping is still updated because a stale resolution would send a later rewatch to the vanished cache path.

### One adoption seam for local copies

**Rule:** every channel through which a local copy turns up — resolve, scan adoption by hash, a completed browse import, and a manual mapping — funnels through one adoption seam that cancels the redundant peer download; a manual mapping joins the seam only once its background hash confirms the content; see [design.md](design.md#download-cache-and-retention).

**Why:** a file being fetched from peers can land locally through another channel mid-transfer (a bittorrent download racing the prefetch, a copy dropped into a media root). If each channel cancelled the download on its own, one would eventually forget to, and the client would keep fetching bytes it already had. A single seam makes the cancel unskippable. The manual mapping is the exception on purpose: it is filename-trusted for the user's own playback, so it resolves Ready immediately, but a mapping to a different encode must never cancel a good download — the download is still the only route to the real bytes — so adoption waits for the hash to prove the content matches. A browse import cancels before placing its payload because both share the same hash-addressed cache path, and an import of a file already held under a media root finishes against the library copy so as not to demote a permanent library file into a retention-evictable cache row.

### Prefetch anchored at now-playing

**Rule:** a downloading client wants every unwatched playlist entry plus now-playing; fetch order is anchored at now-playing (ahead nearest-first, then behind nearest-first) at the chunk level; watched entries ahead of now-playing do not prefetch; NotWatching series are not auto-downloaded; seeders fetch everything with watched back-catalog last; see [design.md](design.md#download-cache-and-retention).

**Why:** the goal is that next week's episode — and the whole queue behind it — is local before the session starts. Anchoring at now-playing puts the bytes that will be needed soonest first. Applying the order at the chunk level rather than per file means the per-source request window is one shared budget: a now-playing advance or a playlist edit re-targets running transfers within a tick, with no cancels and no restarts. There is no point fetching a show the user has opted out of, so NotWatching series are skipped (a local NotWatching file still loads; you can mute).

### BitTorrent is browse-only (2026-08)

**Rule:** BitTorrent serves only the Playlist pane's explicit browse search; missing playlist files are never fetched by torrent; nothing torrent-related survives a restart; see [design.md](design.md#bittorrent-downloads).

**Why:** an earlier torrent-first automatic fetch path was removed in 2026-08 once the relayed peer transfer matured. For rare files, a manual search plus a full BitTorrent client covers the gap. DessPlay is not primarily a torrent client, and keeping the footprint session-scoped keeps it from behaving like one.

### Torrent setting: default off, asymmetric lifecycle, no seeder path

**Rule:** `torrent_enabled` defaults off; enabling applies at startup, disabling applies immediately (seeding torrents removed, pending imports cancelled, completed cached copies untouched); seeders run no torrent path; see [design.md](design.md#bittorrent-downloads).

**Why:** the engine opens ports and joins the DHT, so it must never start unless the user opted in. Disabling has to apply immediately because it is the mid-session escape hatch for a saturated uplink — torrent traffic can drown CRDT sync, and the connection-health line's suggestion points the user here. Enabling only at startup is a simplification: the engine is constructed once or never. Completed imports are untouched because they were hardlinked into the hash-addressed cache at verification and are ordinary cache files by then. Seeders have no torrent path because the browse import is an interactive feature, and a file nyaa can supply makes the seeder redundant; the seeder's job is the rare, peer-only files.

### Session-only torrent seeding

**Rule:** a completed import seeds from its import directory until the app closes, the cached file is evicted, or the setting is disabled; seeding never resumes on the next launch; nothing about a torrent is recorded in SQLite; `<cache>/torrents/` is swept at startup; see [design.md](design.md#bittorrent-downloads).

**Why:** a session typically lasts long enough to clear a 1:1 ratio on a release worth importing, so seeding for the session is a fair contribution without persistence. "The video player is seeding last week's torrents" is unexpected behavior for something that is not primarily a torrent client, so seeding deliberately does not resume. Running the engine with no persistence and no SQLite rows is what makes the startup sweep safe: the only directory spared is one still hosting a registered cache file (the rare failed-hardlink fallback).

### Local-copy offer: evidence classes and trigger (2026-08-31)

**Rule:** with auto-download off, a now-playing file that resolves Missing offers same-episode and near-name local copies for manual mapping; the trigger is derived from state, fires once per file per session, and defers the unknown-series auto-NotWatching write while open; see [design.md](design.md#bittorrent-downloads) and [the proposal](proposals/2026-08-31-local-copy-offer.md).

**Why:** the motivating case is two valid encodes under one filename — a hash mismatch, not just NotFound, so both count as Missing here. The "same episode" class reuses the episode browser's copy-grouping equivalence, `(series id, parsed episode number)`, because there is no AniDB episode id in the schema. The "name match" class is guarded by the filename episode parse because raw Levenshtein rates `- 01` against `- 02` at distance 1 and would happily offer the wrong episode. Deriving the trigger from state rather than hooking the advance event is what makes every arrival channel land on it (EOF advance, manual select, startup with the file already missing, a mapping pruned mid-session) without each needing its own hook. The auto-NotWatching write is deferred while the offer is open so the user is never marked NotWatching underneath a dialog asking whether they want to watch their own copy.


### Directory hint as the AniDB-miss series name

**Rule:** When AniDB does not know a file, the fallback series name is the requester's title-like containing-directory `series_hint`, else the filename stem; see [design.md](design.md#parsing-files-to-seriesseasonepisode).

**Why:** Without the directory hint, per-episode filenames each parse to a distinct series name, so a series' AniDB-unknown episodes split into one franchise per episode instead of grouping under the folder they share. The hint is computed client-side (only the client has the path) and stored once per `anidb_queue` row as the first non-null value reported.

### Re-validation ladder anchored on file age

**Rule:** The never-seen re-validation ladder's age is the *older* of the row's `first_seen` and the file's mtime; clients supply the mtime and the server only ever lowers it; see [design.md](design.md#parsing-files-to-seriesseasonepisode).

**Why:** Anchoring on `first_seen` alone keeps files owned for years on the aggressive new-file cadence: a queue reset stamps every long-owned unknown file with a fresh `first_seen` and re-polls it every 30 minutes indefinitely. The file's own mtime is the honest age. A request without an mtime (a playlist add from a client that doesn't hold the file) must never *raise* the stored value, or a later hint-less request would undo the anchoring.

### Startup reconciliation of settled AniDB rows

**Rule:** At startup the AniDB worker re-arms any `anidb_queue` row marked `has_data` whose hash has no metadata in the loaded CRDT state; see [design.md](design.md#parsing-files-to-seriesseasonepisode).

**Why:** The queue attempt (settled, re-check in a week) is written to SQLite at once, but the metadata write lands only in the periodically-snapshotted CRDT state. A restart in that window loses the metadata yet keeps the settled queue row, orphaning the file (no metadata, no retry for a week). NoData rows are left alone because they self-heal on their short ladder anyway.

### Directory-hint reconciliation each worker pass

**Rule:** Each worker pass rewrites a filename-derived `series_name` to the row's learned `series_hint` when they differ, without an AniDB call; real hits are never touched; see [design.md](design.md#parsing-files-to-seriesseasonepisode).

**Why:** The fallback name is written once, at the first lookup, using whatever hint the row holds *then*. But the hint can arrive after that write: a playlist add carries no hint (the client may not hold the file) and races ahead of the hinted library scan, so the first-seen episode of a series could be frozen with its per-episode filename stem and split into its own franchise. Reconciling on every pass, independent of the settled lookup schedule, closes the race; skipping names that already match makes it quiesce.

### Only structural relations merge a franchise

**Rule:** Only sequel/prequel, alternative-version, and side/parent/summary/full-story edges (`RelationKind::groups_franchise`) merge two series into one franchise; crossover and shared-universe edges are ignored; see [design.md](design.md#parsing-files-to-seriesseasonepisode).

**Why:** Crossover and shared-universe edges — same setting, shared characters, music videos, AniDB's catch-all crossover code — link related but *separate* works. Without the filter a single crossover like *Isekai Quartet* (which relates to Overlord, KonoSuba, Re:Zero and Youjo Senki) would collapse every show it touches into one giant component.

### Name search through the titles dump

**Rule:** The AniDbSearch modal is answered from a locally stored copy of AniDB's daily anime-titles dump (case-insensitive substring over all titles and synonyms, ranked exact > prefix > substring, one hit per series), as plain wire messages rather than CRDT state; see [design.md](design.md#parsing-files-to-seriesseasonepisode).

**Why:** The UDP API has no multi-result search — `ANIME aname=` is an exact-title lookup, useless for informal names like "GochiUsa". The titles dump is AniDB's sanctioned approach for name search, at most one download per day. Search results are transient request/response data with no reason to be replicated.

### Drag-in adoption is filename-trusted

**Rule:** A file the user loads directly into mpv whose basename matches the now-playing entry is adopted as a manual mapping with no hash check; see [design.md](design.md#manual-file-mapping).

**Why:** It is the same "the user explicitly chose this file" exemption the browser map gets (see Content Hash). The trade-off — a same-named *different encode* dropped in silently desyncs that client from the group — is accepted for parity with the browser map and because the user deliberately loaded that exact file. A mismatched mapping is still never *served* (`CannotServe`), so the damage stays local. The route is especially handy in attach mode, where driving mpv directly, including dragging files in, is the normal workflow.

### Mismatch re-check watcher

**Rule:** A name-matched file that fails the hash is polled for `(mtime, size)` about once a second and re-resolved once it has changed since the failed hash and then held still; an unchanging mismatch is never re-hashed and its watch expires after 10 minutes; see [design.md](design.md#content-hash).

**Why:** A name-matched file that fails the hash is usually a copy or external download still being written into a media root — the hash ran mid-write. Watching the path flips the entry to Ready seconds after the write finishes rather than at the next library scan a minute later. A genuine different encode never changes on disk, so re-hashing it would be wasted work; its hash-cache row still matches the disk, and the periodic scan remains the long-tail safety net.

### Personal and group watch records are separate

**Rule:** Personal watch history (local SQLite, 85% rule) and the group's synced watched flag (server-written at EOF) are tracked separately and used for different things; see [design.md](design.md#watch-tracking).

**Why:** The personal record is keyed by hash/series so it survives cache eviction, and it answers per-user questions: recency sorting, unwatched filtering, which *copy* of the previous episode this client played, known-series detection, and the auto-archive trigger. The group flag is the shared answer to "where are we?": play-history muting, "behind the group" eviction, and The List's position — so a user who misses a session still sees the group's progress, not their own.

## Player Integration

### keep-open always and autoload disabled

**Rule:** mpv is launched with `--keep-open=always` (not `yes`) and `--script-opts-add=autoload-disabled=yes`; see [design.md](design.md#player-lifecycle).

**Why:** `always` parks the file at EOF regardless of playlist length. User scripts such as autoload.lua pad mpv's playlist with sibling files, and `yes` would auto-advance into one, hijacking end-of-file and skipping the group forward. The script-opt additionally switches autoload off so stray playlist-next keys typed into the mpv window find nothing. The user's mpv.conf is otherwise honoured (no `--no-config`).

### Crash ladder escalation

**Rule:** Player deaths within 30s of each other escalate: relaunch silently, then pause globally with a shared chat message, then give up until a different file is loaded; see [design.md](design.md#player-lifecycle).

**Why:** A file that reliably kills the player would otherwise loop forever, spamming the log and re-pausing on every death — hence the give-up step, with a different now-playing as the deliberate recovery action. The second-death notice is a real synced chat message rather than a derived system line because a crash is the one state change peers cannot derive from their own view (they have no signal for *another* user's player dying), so it must be communicated; being an ordinary chat message it also persists and reaches late joiners. The relaunch after the second death comes up paused because that is the safe state if the file itself is crashing the player.

### Observed pause re-anchors the position estimate

**Rule:** An observed pause is followed by a `get_property time-pos` query whose reply re-anchors the position estimate on the frame mpv actually stopped at; see [design.md](design.md#events-from-player).

**Why:** Otherwise the wall-clock-extrapolated estimate counts the observation's in-flight window as phantom playback, and a paused mpv emits no further `time-pos` changes to correct the overshoot.

### File attribution gated on the observed path

**Rule:** File-attributed observations (position, seek, EOF, duration) are accepted only while the last observed `path` equals the commanded file, and drift correction is suspended while the player is off that file; load-failure reports and programmatic-seek echo accounting are exempt; see [design.md](design.md#events-from-player).

**Why:** `loadfile` is asynchronous: after a load is commanded, mpv stays on — and keeps reporting positions, seeks, even the EOF of — the *previous* file until the new one actually opens, and on a slow machine (cold NAS, heavy mpv scripts) that window is long. mpv's events carry no file identity, but its IPC event stream is ordered, so the observed `path` closes the window. Suspending drift correction in the gap means a mid-load window, or a file the user dragged in themselves, is never slewed or hard-seeked.

The two carve-outs: a file that fails to open may never produce a path observation at all, so gating the load-*failure* report would suppress it entirely — and a stale one merely re-resolves the file (wrong but self-healing, the safe direction). The echo accounting of a programmatic seek is consumed even when the echo arrives gated-out because it is our own seek; leaving it outstanding would swallow the user's next genuine seek as a stale echo. Only the user-seek/debounce half of seek handling sits behind the gate.

### The dessplay profile and command replies (2026-09-06)

**Rule:** A `[dessplay]` mpv.conf profile is applied once per IPC connection via the `apply-profile` command (not `--profile=dessplay` on the command line, and not per file); the reader consumes mpv's reply to every command and logs rejections — both outcomes of `apply-profile` — at debug; see [design.md](design.md#player-lifecycle).

**Why:** The command-line form makes mpv exit with status 1 when the profile doesn't exist, punishing users without one; the IPC command merely fails. Per-connection (rather than per-file) application suffices because `reset-on-next-file` is cleared, so profile-set options survive `loadfile` — a real-mpv sequence test pins this, including the placeholder-first opening (testing-strategy.md, Real-mpv tests). The reply readback exists because of a field report (2026-09-06): a user's mpv script saw no dessplay profile on the playing episode, and the logs could not distinguish "never applied" (a typo'd profile, none at all) from "applied and later reset" — command replies were written fire-and-forget and mpv's answers discarded. Replies now settle a pending-command table keyed by request id; failures log at debug (not trace, which is off in most field logs). The report itself reproduced as neither: the sequence test shows the profile holds, pointing at the script not observing profile changes.

### Collapsing incremental subtitle cues

**Rule:** A subtitle observation that is a prefix or suffix of the previous line replaces it in place (growth) or is dropped (shrink-back); see [design.md](design.md#subtitle-display).

**Why:** mpv re-emits the whole joined on-screen value on every change, so the on-screen cue-set evolving produces observations that are the *same* utterance, not a new line. Some subs reveal a line letter-by-letter over 2–3s as rapid-fire cues; when two ASS events display at once mpv joins them with a space, and as one ends the text shrinks back to just the other, from either end since mpv's join order is not fixed. Without collapsing the shrink-back, a brief interjection overlapping a stable line duplicates when it clears. The known cost — an unrelated later cue that happens to be a prefix or suffix of its predecessor is collapsed too — is rare and accepted; no time-window guard.

## Data Storage

### Local and synced state in separate databases (2026-08-21)

**Rule:** Local-only data lives in `dessplay.db`; the replicated CRDT snapshot lives in the derived sibling `dessplay.sync.db`, which is disposable; see [design.md](design.md#sqlite-database).

**Why:** The sync file's contents are a replica of server-authoritative state, so resetting wedged sync state (`--reset-sync`, `/resync`) should never cost local data such as the hash cache, watch history, or manual mappings. Before the 2026-08-21 split both lived in one database and a reset was destructive. The first-open migration moves the legacy `crdt_state` row over and drops the old table, idempotently and crash-safely.

### Unsent local ops are not persisted

**Rule:** Local ops the server has not yet seen are buffered in memory only; a crash loses the most recent local edits; see [design.md](design.md#sqlite-database).

**Why:** Crashes should be rare enough not to matter, and an edit that *caused* a crash should not be replayed into the next session. A persisted op log would buy durability for exactly the edits most likely to be poisonous.

### Tagged snapshot envelope

**Rule:** The stored `crdt_state` blob carries a 4-byte magic plus the protocol version ahead of the postcard body; one untagged legacy layout (v6) is decoded and migrated, byte-identical tagged versions decode via an explicit compatible list, and anything else is refused; the server backs up its database before first persisting a migrated blob; see [design.md](design.md#schema).

**Why:** A blob should name its own layout instead of being identified by trial decode — postcard will happily "succeed" on the wrong layout. The first byte 0xFF is chosen because no untagged postcard state can start with it, which is what makes the single legacy arm safe. Refusing unknown versions matters most for the *server*, which is authoritative and cannot re-sync its lost state from anyone; a deliberate migration adds an explicit decode arm instead of guessing. An interactive client has the cheaper fallback of dropping an unreadable blob and re-syncing from the server.


## A local expedition for the waiting room (2026-09-05)

The Waiting Below gives people something to play while friends arrive. Five
floors, a recover-and-return objective, fog, finite supplies, and body-part
injuries provide a small complete roguelike without introducing another
multiplayer protocol. Turns depend only on explicit commands: five-minute
sessions must not punish someone for leaving to watch an episode. The
existing log-modal layout keeps party chat visible, while a sticky presence
banner makes arrivals noticeable even during help or after a death.

The session bridge owns persistence and the UI only displays committed
results. Saving after every action, including the RNG state, makes closing,
crashing, and reopening ordinary lifecycle paths rather than special game
save operations. SQLite transactions also contain the finished-run history
and an outbox report. A disk failure cannot leave an advanced UI paired with
an older save. Invalid/future saves are preserved for recovery rather than
silently starting over. Saves are per username in the irreplaceable local
database, so a sync reset does not erase them.

Death summaries are real synced chat, because a late-arriving friend should
see how the expedition ended. Local narrator lines cannot provide that.
Their saved timestamp, sender, and expedition-numbered text form a stable
retry identity. The sync actor deduplicates and flushes before acknowledging
an outbox record; a crash between the two databases therefore safely retries.
This adds a local command, without changing network or CRDT schemas.
Automatic reports do not imply returning from Away and do not enter IRC.

The command popup derives its height from the filtered command table. Adding
`/rogue` exposed the old fixed fourteen-row cap, which hid `/quit`; removing
that independent cap makes future commands discoverable without a second edit.
The whole-app test requires every command to render on a sufficiently tall
terminal.


## Lasting injuries and an awakened dungeon (2026-09-06)

**Rule:** Survival without the ember is a victory; committed ember attempts
should usually fail. Taking it explicitly and irreversibly awakens a dangerous
ascent with breaches, swarms, warned collapses, and lulls. See
[design.md](design.md#the-waiting-below).

**Why:** The initial playtest reached the ember in good condition, then spent
393 of 935 turns returning through almost entirely cleared floors. Shortening
the ascent alone would discard a useful source of tension: remembering a
place that has become dangerous. We instead preserve the recognizable layout
and transform it. Warnings make committing to a route consequential; lulls
give wounded survivors opportunities to treat themselves. Action-time hazards
preserve the waiting-room promise that a watch-party interruption is safe.
Collapses must preserve an escape route, including dormant cavern entrances;
a property test caught a future breach becoming disconnected after an earlier
collapse removed its mouth, so such mouths remain passable rubble.

Injuries are intended to accumulate into a death spiral. Shared anatomy,
regional armor, different weapon injury types, permanent functional losses,
and rare miraculous fountains give individual encounters lasting costs.
Ordinary care separates stabilizing bleeding and supporting a fracture from
repairing structural damage. Automatically choosing useful care prevents
that complexity becoming mandatory repeated medical administration. Manual
selection remains available for an unusual priority. Blindness reduces current
perception while preserving memory and audible warnings; losing sight must
not make a creature harmless to the rest controller.

Walking preserves breath at best, while sprinting buys speed with breath and
noise. A shared integer action clock makes walking, sprinting, limping, weapon
recovery, physiology, and enemy windups comparable without letting faster
commands evade injury costs. Sparse, separated early encounters provide room
to learn and retreat; later threats and the awakening retain severe injury
consequences. Observation-limited surveys informed two density passes, but
scripted-policy outcomes are diagnostic evidence, not human win rates. The
balance direction does not specify a promised percentage.

The journal is the durable explanation of danger, injuries, and care. Red
flashes and decorative corruption supplement it without obscuring controls
or changing mechanics. One observation interface serves the modal and agents,
so disabling effects cannot grant agents extra knowledge or healthier bodies.
UI-paced rest sends one ordinary saved action at a time and waits for its
acknowledgement; cancellation clears future scheduling, even when the already
accepted action later replies.

One manual return crossed the heavy-load threshold just before taking the
ember. Its extra protection helped, but walking took 150 rather than 100
time and the player had not appreciated that tradeoff. Equipment inspection
and the plain harness therefore show actual walking time, sprint time, and
sprint breath cost for the current body and kit. These are decisions a player
can act on, not merely raw armor values.

**Compatibility decision:** Nobody besides the author had played this
pre-release game. The user explicitly approved deleting its database section
instead of migrating old characters and layouts. Local migration v8 therefore
clears both roguelike tables, including local history and pending reports,
in one transaction, then normal version-2 saves apply. Other local records
and already-published chat remain intact. This is a single upgrade operation,
not a policy of silently resetting malformed or unsupported saves afterward.


## Dungeon controls should express intent (2026-09-06)

The author's first hands-on feedback found that a single side wall blocked
an otherwise natural diagonal step, and that a starting spear's “reach 2”
stat did not explain how to use it. A diagonal now requires one open flank,
with the same rule for movement, sight, attacks, doors, and creature paths.
Two blocked flanks still prevent squeezing through a closed corner.

Moving toward an enemy now uses the weapon's available reach automatically.
The player expresses an attack by moving toward the visible threat; they do
not need a separate input sequence to receive the spear's benefit. Explicit
attacks remain for deliberate probing, and sprinting still expresses movement.
The automatic choice cannot discover unseen targets or use a spear without
the necessary hands. Weapon descriptions and an equipped-spear hint explain
that a thrust across an empty tile does not move the character.

Routine rest narration was filling the journal with identical lines. Adjacent
identical recovery events now coalesce, including when displaying older saves;
physiology still advances and distinct treatment/combat events remain. Ground
items also get a blank separator above them. The inventory renderer and input
handling share a row mapping so that adding space cannot equip the wrong item.

## Creature anatomy and readable wounds (2026-09-06)

**Rule:** Combat names the attacker, its action, and the defender's actual
anatomy. Wound details are qualitative, and the bounded health summary is
separated from threats by a blank line; full details remain under `v`.

**Why:** An ash rat was reported as having arms because shared region IDs
were also used as human-only display names. Enemy retaliation prefixed the
same context-free sentence used for player strikes, making “ash rat: The
torso takes the blow” ambiguous about whose torso was struck. Keeping the
shared injury model while requiring species-aware names fixes the vocabulary
without changing damage, targeting, or saves. Structured impact results let
one narrator describe player attacks, enemy attacks, deflections, and falling
stone with explicit ownership. Bite and crushing mechanics use weapon damage
profiles internally, but neither rat teeth nor a brute's weight are weapons
in the prose. Tool users' windups and attacks also respect lost grip.

The author found “flesh 70/80” too mechanical and requested descriptions such
as “left foot scratched, bone damaged”. Qualitative tissue, nerve, organ,
and local bleeding descriptions preserve actionable distinctions, including
lasting loss and supported versus unsupported fractures. The author chose
to retain the separate status and supply counters. Descriptive severity
bands use species-relative flesh integrity; they do not rebalance injuries.

Shorter words alone cannot guarantee a fitting panel. The sidebar now wraps
injured regions, reserves threat space with the requested blank separator,
and explicitly points to full condition details when entries do not fit.
It omits healthy regions instead of consuming the panel with “sound” rows.
Recovery shares that summary and recent combat logs wrap too. A visual-row
condition cursor makes entries taller than the viewport accessible; each
row carries its region identity so treatment remains correct after wrapping,
resizing, or healing that shortens preceding entries. Existing saved journal
strings are historical records and are not rewritten.

## Template-owned pane composition and drag state (2026-09-07)

**Rule:** Pane focus and mouse targets follow the rendered application template.
Resizable containers own named splits; drag state is stored separately by
source and revision and invalidated before a changed bundle is drawn.

**Why:** Keeping the old four splitter variants beside editable root composition
would make mouse boundaries depend on where panes used to be. Deriving handles
from arranged adjacent children makes reordering and gaps work consistently.
Only the neighboring children exchange shares, so another splitter stays put.
Controller identity remains the semantic slot binding, independent of node IDs.

Saving whole Settings on release could overwrite unrelated preferences and
could not distinguish two layout directories. A dedicated layout-settings
action writes its own storage key in both first-run and session loops. The UI
retries pending revision updates through a full action channel; ordinary
settings writes cannot revive old sizes. Initial legacy import is one-time,
so editing the layout while the client is closed also wins on restart.

## Chat attachment frames and scrolling geometry (2026-09-07)

**Rule:** Layout templates own chat/input/suggestion composition and attachment
chrome. The image's full frame consumes the existing height budget and scrolls
as one measured object; pixels retain their original fitted size.

**Why:** Painting a border around the visible image slice would create a moving
frame on scroll. A signed scene translation with an explicit viewport preserves
original edges even above the viewport origin. The image protocol still owns
pixel encoding and cropping; scene geometry supplies its origin and protection
rectangle. Measuring the timestamp gutter avoids assumptions about timestamp
width and allows border/alignment changes using files alone.

The same viewport now defines chat hit rows, independent of the enclosing
border. Width changes retain a stable message/source anchor instead of retaining
only a visual offset from the changing tail. Recent chat remains a read-only
projection. Subtitle timestamps, optional speakers, and text are separate fields
with stable cue identities rather than preformatted strings.

## Semantic collection fields and measured rows (2026-09-07)

**Rule:** Users/Playlist templates receive unpadded fields and stable row keys.
Collection layout measures row heights after widths and publishes hit regions
with the painted rows. Selection and viewport centering remain independent.

**Why:** A preformatted table line fixes column order before a template can act.
Passing watch labels, download percentages, temporary state, title, and marker
separately allows file-only reordering. An intrinsic shared watch-label width
keeps the default aligned table without inserting alignment spaces in content.

Forcing every template row to one line would make padding and wrapping silently
clip content. Natural-height row scenes reuse the width-first allocator and
signed clipping path. The viewport measures backwards around its center target
and forwards until filled, retaining existing centered selection and now-playing
behavior without constructing an entire collection tree. A continuation's pointer
record names the same controller item as the first line. Styles and layout reloads
leave the selection/controller identities intact.

## Roguelike composition and semantic frame colors (2026-09-07)

**Rule:** The roguelike frame and game page use the shared renderer. Map cells
remain an intrinsic primitive, while captions, metric spacing, and the
map/sidebar/journal split live in templates. Existing injury effects supply a
typed semantic color, overridable by authored custom-property declarations.

**Why:** Leaving the sidebar split in the game controller would prevent the
requested file-only relocation. Labeled metric fields remove fixed alignment
spaces from statistics and supplies. The map's minimum space and bounded recent
journal preserve the default allocation; the same wound/threat policy receives
the sidebar's newly arranged size. Typed color variables let effects retain
their Full/Reduced/Off behavior without mutating CSS or overwriting authored
styles after painting. Bottom captions are explicit frame metadata, so their
bounds and clipping follow the same original-frame paint path as top captions.

## Inline semantic text and recovery overlays (2026-09-07)

**Rule:** Inline flows measure semantic child text as one stream while retaining
field/source/action ranges and resolved span styles. Box and wrapping rules live
on the enclosing flow; inline box declarations are rejected. Centered overlays
reserve a one-cell inset and paint original frame captions independently of
content overflow.

**Why:** Recovery metrics previously became one formatted string, so authors
could not rearrange their fields. Separate flex boxes would lose the existing
word wrapping on narrow screens. Inline flows preserve that behavior while
keeping labels and values independently editable. Typed rich spans use the same
measurement and paint path and carry existing controller action keys rather
than executable expressions. The terminal adapter centers allocated overlay
boxes explicitly: Taffy's absolute auto-margin positioning does not account for
the reserved inset consistently at very small sizes. Properties cover those
sizes and translated origins. Guide/journal text, equipment rows, and expedition
endings also use measured scenes; condition-row and bounded-summary composition
remain tracked work rather than hidden behind a completion claim.

## Attached log dropdowns (2026-09-07)

**Rule:** An explicit `placement="after"` overlay follows its parent control and
escapes that control's clip only within the entry viewport. Hidden ancestors
suppress the popup. Log labels, current values, options, and footer are semantic
fields rather than a preformatted header and a separately positioned widget.

**Why:** Reusing a controller-computed popup rectangle would make file-only
control rearrangement visually detach the options from their trigger. Attached
overlays keep that geometry in the renderer. They are a bounded placement mode,
not arbitrary positioning. The existing scope selection, session-only filter
updates, cancellation, and modal capture stay in the log controller.

## Watch paths and directory replacement (2026-09-07)

**Rule:** Layout watches compare normalized absolute paths and attach to a
surviving ancestor of the override directory. Missing directory suffixes are
resolved against the nearest existing canonical ancestor.

**Why:** Real filesystem regressions failed for atomic saves, relative paths,
and directory replacement, including outside the sandbox. The event backend
canonicalizes paths while the previous filter compared their spelling with the
original argument; on macOS temporary-directory aliases also hit this class.
Watching the selected directory itself additionally couples the watch to an
inode that an atomic bundle replacement can remove. Normalizing the filter and
watching the parent fixes both paths. Tests wait on actual compilation delivery,
without arbitrary sleeps, and cover initial absence and later replacement.
The six file-only authoring demonstrations now install their results through
this actual watcher rather than relying only on direct compiler calls.

## Controller slots share template layer order (2026-09-08)

Painting base chrome, then all controllers, then overlays erased controllers
placed inside an authored overlay. The same split existed in chat, collections,
log controls, dungeon inspection, subtitles, and application composition. All
now use the renderer's ordered slot callbacks; forms and browser dialogs already
used that path. The split-layer convenience API is removed. Controller roots
publish their actual bounds so hiding the root also removes its focus target.
Selection enters the cascade before CSS, since a final reversal pass prevented
authors from controlling selected appearance. Unused legacy list/table painters
were removed to keep one production allocation and paint path.

## Frame-wide graphics suppression and inline inspection (2026-09-08)

A child pane can contain an authored overlay that overlaps chat even when the
application template contains no overlay. Immediate graphics emission could
therefore bleed through a later pane. The renderer now records painted overlay
regions across composed scenes and queues image operations until the full frame
is known. Overlapping operations are dropped; normal modal suppression still
avoids allocating image rows. The queue belongs to the UI-thread renderer, not
the controller, and controller ownership remains transferable before startup.

Inline flows retain definition metadata and fragment rectangles for F12. They
do not create independent boxes or wrap again for inspection. The offline
layout_smoke example uses the production terminal setup, watcher, and UI loop
so graphics checks need neither a rendezvous server nor a player.

Pointer targeting follows recorded pane paint order in reverse. The prior
fixed Chat/Series/Users/Playlist search selected an obscured pane when authors
overlapped grid cells. Click and wheel paths now share the ordered hit lookup,
including the nonfocusable subtitle viewport; modal capture remains unchanged.

## Authored virtual collections and explicit visibility (2026-09-09)

Eager repetition was appropriate for category tabs and health fields but could
not expose full collections safely. Users, Playlist, and Series now supply raw
keyed rows to virtual repeats. A repeat builds a window sized from the terminal
height plus overscan, then centers using actual measured heights. Nested local
scenes retain signed scrolling transforms so clipping never changes wrapping,
source identities, or original borders. Existing body-slot customizations keep
using the shared collection renderer; displaying both forms of one controller
is rejected by the compiler.

A viewport window based only on model indices was underfilled by sparse lists
whose conditional children were empty. Content-existence probes cache by states,
conditions, and text emptiness before choosing the window. These probes do not
use clipping to decide whether an item exists. The shared cursor receives the
explicit hidden indices, also for dialog body slots, so offscreen choices remain
navigable and completely hidden choices cannot be activated. Save remains a
global form command. Editors retain their own row identity across those changes.

Search and name editors still owned a border inside their template slot. Their
remaining wrapper now paints only styled editor contents; the shipped stylesheet
owns the same default border. Focus fallback likewise needs its appearance in
the frame that publishes the new target: a bounded second composition pass runs
before image emission when fallback changes focus, preventing a stale highlight
and keybar without allowing authored focus styles to create an unbounded loop.


## Nyaa search progress, history, and multiple selection (2026-09-14)

The downloader now reports RSS fetching followed by measured metadata-inspection
progress. The denominator is the capped feed prefix, and every inspected entry
advances it, even when unseeded, malformed, unavailable, or a multi-file batch.
A percentage during the RSS request would invent progress, so that stage has an
explicit status label. Request identities, rather than query text alone, keep
late replies from a closed dialog or a repeated query from changing current work.

Repeated episode searches benefit from shell-style recall: an empty query shows
recent searches, Up/Down recalls into the existing line editor, and Down past the
newest restores the draft. The latest 100 distinct trimmed submissions persist
locally, with repeats moved to the front. This is query convenience data, separate
from both editable settings snapshots and the torrent engine's session-only state.
Selecting a history entry permits editing before any network request.

Checkboxes let users pick several individual releases without weakening the
single-file metainfo filter. Space toggles, Enter downloads checked rows, and
with no checks Enter retains the existing single-highlight behavior. Tab returns
to query editing, allowing spaces without toggling a result. Edits clear all old
results and checks. The existing import engine already supports independent jobs;
the UI emits one action per selection and registers every job immediately so
reopening permits cancellation even before the first actor progress report.
Downloads retain the original playlist anchor and publish independently when
verified; waiting for a slower selected download would unnecessarily delay use.


## Nyaa results require explicit query editing (2026-09-14)

The initial multi-selection UI advertised Tab for query editing but still let
unhandled keys and pastes fall through to the editor, discarding checked results.
Results now own input until Tab returns to editing. One editability predicate
gates the entire editor and its visible cursor, covering typed characters,
paste, deletion, and readline shortcuts together. Active imports already exclude
the editor; history, empty results, and errors remain directly editable because
there is no result selection to lose. AniDB's separate search deliberately keeps
its existing type-to-rearm behavior.

A whole-app property regression was run and confirmed to fail on one typed `x`
before the fix. It exercises mixed editing events without changing results or
checks, verifies Tab restores editing with the original cursor, and checks that
a subsequent response captures input again while checkbox/download keys work.


## Chat search uses ranked results (2026-09-15)

Chronological match navigation privileged newer scattered-letter matches over
older exact words, discarding the shared matcher's relevance order. Chat now
uses the same ranked collection presentation and navigation as other panes.
The shared search controller has no source-order mode, so callers cannot
accidentally discard relevance. The conversation is hidden during search;
selection does not move its source anchor. Enter restores chronological context
and centers the accepted stable message identity using the current viewport,
including after a resize. Esc retains the prior reading position and draft.

The one-second query delay remains, with navigation and Enter applying pending
edits immediately. Incoming messages preserve the selected identity and applied
query. Wheel movement selects results, and hidden conversation hit records and
image operations are absent from result rendering. Result bodies keep spoiler
concealment; Enter restores the full message presentation and image previews.
Series, Users, Playlist, Subtitles, and Logs already use relevance-ranked
collections; file browsing ranks through the same scorer with directory grouping.
The chat-specific chronology exception was the only sibling to remove.


## One Anthropic model constant (2026-09-23)

**Rule:** All Anthropic calls use `dessplay_core::ai::ANTHROPIC_MODEL`
(`claude-opus-5-5`), each with an explicit effort; see
[design.md](design.md#anthropic-model).

**Why:** Commentary and the curator had each hardcoded their own model
and had drifted apart (`claude-opus-4-6` and `claude-opus-5`). With one
constant, a model bump is a single edit that can't miss a feature. The
constant sits in `dessplay-core` because the curator runs in the
rendezvous crate and the other features run in the client. Only the
constant is shared across crates. The client-side transport
(`dessplay/src/anthropic.rs`) is shared by commentary and the oracle.
The curator keeps its own transport because it needs different timeout
handling. Opus 5.5 can't disable thinking and defaults to `medium`
effort, so effort is always stated explicitly: `low` for commentary and
the curator. Commentary's `max_tokens` rose from 3000 to 8000 because
thinking counts against it.


## Oracle is a separate headless node (2026-09-23)

**Rule:** The oracle is its own stateless headless process
(`dessplay --oracle`) on the rendezvous host. It connects in the Seeder
role and reads its API key from the environment; see
[design.md](design.md#oracle).

**Why:** Running the feature in every client would need every client to
hold a key, and would need an election so only one of them answers. One
node with a server-held key (agenix, the existing `claude-api.key`)
answers exactly once, and it is up whenever the server is. We reused the
Seeder role instead of adding a `Role::Utility`, because a new variant is
a wire change that older clients can't deserialize. The roles already
mean the right thing: headless and never gating playback. The cost was
that seeders were excluded from chat tab-completion. That exclusion was
dropped, since completing the colocated seeder's name is harmless and
completing `oracle: ` is the point.

## Oracle answers only questions that arrive while it watches (2026-09-23)

**Rule:** The oracle baselines the first adopted chat view, dedupes lines
by content rather than list position, and ignores questions older than
5 minutes with a cutoff that never moves backwards; see
[design.md](design.md#oracle).

**Why:** A stateless client re-adopts the whole chat on every restart and
reconnect. Answering from that would replay old questions. List indices
are not stable, because GList merges can insert mid-list. The age cutoff
covers a line that surfaces late after a partition, when the asker has
long since moved on. It also bounds the seen-set's memory. The cutoff
never moves backwards so that a local clock step can't revive a line
that has already aged out. The property test
`each_fresh_question_is_answered_exactly_once` fails when the baseline
is removed.

## Oracle uses server-side web tools (2026-09-23)

**Rule:** The oracle grounds answers with Anthropic's server-side
`web_search_20260209` and `web_fetch_20260209` tools. It uses a combined
budget of 16 calls per answer, and exhaustion is signalled by a
mid-conversation system message, not by changing the tools; see
[design.md](design.md#oracle).

**Why:** The server-side tools fetch and condense pages on Anthropic's
side (dynamic filtering), so we have no HTML-to-text dependency and no
scraping code to maintain. `max_uses` applies per request, and every
`pause_turn` continuation is a new request, so the loop enforces the
budget itself. Changing `tools` mid-answer would break the
byte-identical prefix that prompt caching and preserved thinking rely
on. A system message appended after the paused assistant turn leaves
the prefix intact. The wrap-up request is the last one sent, so an
answer costs a bounded number of requests even if the model keeps
searching.

## Oracle marks now-playing switches in its chat window (2026-09-26)

**Rule:** The oracle interleaves a `--- now playing changed from X to Y`
marker into the chat lines it sends, at the switch's LWW stamp; see
[design.md](design.md#oracle).

**Why:** The group keeps talking about the previous episode for a while
after moving on. With only the current episode in `<now_playing>`, the
model has no way to tell that a question refers to the earlier one. A marker inline in the chat says exactly
which lines came before the switch; a separate list of switches would
leave the model to line up timestamps itself. The stamp comes from the
`now_playing` register (exposed as `StateView::now_playing_since`), which
is the same Lamport shared clock chat uses. Observation time would land
a network hop late and put the marker after lines typed just after the
switch. The shared state only holds the current file, so "from" is known
only to an observer: the oracle records switches as it watches them and
caches each file's label while it plays, because the old entry may leave
the playlist in the same update. It persists nothing, like a seeder, so
a restart forgets earlier switches. Its adoption state is a baseline,
the same as for chat questions.

## Houseguest is a post-render overlay (2026-09-28)

**Rule:** The idle houseguest paints over each finished frame from the
shell loop, reading the frame and a read-only `IdleView`; nothing flows
back into `Ui`. See [design.md](design.md#houseguest).

**Why:** She exists only while nothing else is happening, so wiring her
into the layout renderer or tui-realm (message variants, focus, template
slots) would spread a toy across the core UI. As a layer between the
renderer and the terminal she is one module plus a few shell lines, and
deleting her deletes the feature. Leaving is a dissolve rather than
"dragging everything back", because an undo animation that respects the
user's time can't be done in the 2–3 s a returning user will tolerate.
Rejected: moving real UI state for her gags (it would sync); behaviour
trees or async scripts (they hold mid-plan state an interrupt must
unwind); a fixed frame rate (CPU while she stands still, and pacing tied
to redraw speed).

**Remote chat makes her look, not leave (2026-09-28):** a friend's
message arriving while you're away shouldn't end the visit you'd come
back to; only local input means you're back.

## Houseguest terrain comes from the rendered frame (2026-09-28)

**Rule:** Floors, poles, and drop-offs are derived from the finished
frame's cells (box-drawing runs), with protected rectangles solid. See
[design.md](design.md#houseguest).

**Why:** Layouts are user-authored, so pane rectangles don't say where
the borders are drawn, whether a pane has one, or where a title
interrupts it. Reading the cells makes any layout walkable and picks up
floors no rectangle knows about (the status separator, chat day rules,
which scroll away and make her fall). The floor may lie inside a
protected rectangle (she stands on the status separator); only her body
cells are checked. A release-build read of a 200×60 frame takes ~80 µs,
so it runs every frame she is on screen.

## Houseguest line art over kitty placeholders (2026-09-28)

**Rule:** With kitty graphics, Osaka is line art placed by unicode
placeholders; her box only covers blank cells and solid lines her image
redraws. See [design.md](design.md#houseguest).

**Why:** A 5×4-cell ASCII figure read as "a very large-headed alien";
everyone in the group runs Ghostty, so line art costs nothing in reach.
Placeholders replace the cells they cover — transparent pixels show the
cell background, not the text — so rather than float over text (which
would mean hand-rolling kitty placements outside ratatui-image, or
re-rendering the text ourselves), she only stands where nothing would be
hidden, and redraws any border line under her. That also puts her feet
*on* the line instead of half a cell above it. Her exit keeps two image
beats and then bursts into text rain, because a stream of per-frame
images would pile up in the terminal's image store (ratatui-image never
deletes images). Rejected: vendoring ratatui-image for real overlay
placements (possible later); a larger box (more detail, fewer floors on
80×24).

## Houseguest mischief undoes itself on a schedule (2026-09-28)

**Rule:** Letter swaps and sneeze scatter schedule their own undoing when
they're made, independent of her current act; a chat arrival undoes them
at once. See [design.md](design.md#houseguest).

**Why:** Her acts are interruptible by design (a chat message turns her
round, a pull can lose its grip, a resize re-anchors her), and each of
those clears what she was doing. Keeping the undo in the act meant any
interruption could leave `teh` on screen until the goodbye — mischief
that silently becomes a wrong display. A separate queue of dated layer
changes survives every interruption and makes "self-reverting" hold by
construction. Undoing at once on chat keeps the rule that someone else's
conversation is never changed while they're around: the newest line is
protected anyway, but swapped letters two lines up would still read as
a garbled log. Rejected: undoing only at the goodbye (the swap would last
minutes, and it isn't a joke any more); swapping within the newest line
(it's the one people are reading).

## Houseguest swaps letters where they are shown (2026-09-28)

**Rule:** A pulled line is never pulled again, but letter swaps read the
frame with her layer painted, so a pulled line's letters can be swapped
where they sit; the undo puts each letter back where it was shown. See
[design.md](design.md#houseguest).

**Why:** Pulling the same line twice looks robotic, but after a pull the
line she moved is usually the only text in reach — it ends at her hands
— so hiding it from swaps too left her with nothing to play with. The
layer operations now name a glyph by its source *and* where it's shown:
a swap trades shown places, and the undo is "put these back where they
were", which is home for an ordinary swap and the pulled spot for a
pulled one (undoing to home would half-unpull the line). The same
primitive serves the sneeze put-back. It only ever moves glyphs already
in the layer, so an undo after the text changed underneath can't pick up
new text. Rejected: re-deriving the swap against the real frame
(the pulled letters' real cells are holes).

## Houseguest furniture is pane-relative and leaves with her (2026-09-28)

**Rule:** A piece of furniture is stored as a quiet pane and a fraction
along its bottom border, resolved against every frame; it shows only
over blank cells and while she is there, and the goodbye rain takes it
along. Pieces are drawn filled, in her style. See
[design.md](design.md#houseguest).

**Why:** Absolute cells go stale on the first resize or layout edit, and
panes are what the user recognises as places ("her sofa is in the Users
pane"). Blank cells only, because a kitty placeholder hides what's
under it, and furniture over a user's text would be a screensaver
getting in the way. Leaving with her follows from her being a
post-render overlay: once the user is active the screen is theirs, and
a sofa that outlived her would be a UI element nobody asked for. Filled
art: the user compared filled and coloured-outline versions on the
props sheet; the fills are what read on the dark theme and match her,
while outlines lost the TV screen and turned the quilt to noise.
**Panes are rooms** (the user's call): pieces carry a room tag so the
sofa and TV stay together and the bed and desk make a bedroom elsewhere
— a home reads as rooms, not furniture scattered wherever it fits. A
room that loses its pane moves whole (the user preferred that to pieces
waiting in the closet one by one); text over a piece doesn't move the
room, only closets that piece, so a user joining the Users pane doesn't
send her living room wandering. Later, she should be able to *make*
space for her things; until then, no space means the closet.
*(2026-10-03: superseded. Rooms come from contents since
[2026-10-02](#her-home-stands-on-strips-and-its-rooms-are-whats-in-them-2026-10-02):
a strip's role is what's unboxed there, and pieces carry no room tag.
Keeping the sofa with the TV and the bed out of the living room is
phase 4's rules and repairs
([2026-10-03](#she-puts-her-home-right-with-the-piece-in-her-pocket-2026-10-03)).
What stays is the moving whole, now per strip.)*
Rejected: the chat as a room (new lines would send pieces to the closet
constantly); putting her "room" in a fixed screen corner (layouts
differ, and the corner is often text).

## Houseguest and her furniture share one image (2026-09-28)

**Rule:** Furniture is solid to text but not to her: she walks in front
of it, and wherever her box overlaps pieces, one image draws them and
her back to front. See [design.md](design.md#houseguest).

**Why:** It's image composition, nothing more (the user's call, after
the first attempt over-engineered it). Two kitty images over the same
cells would each blank the other's cells with their placeholders, so
the overlap has to be one image. The cost is distinct images — every
column she crosses in front of a piece is a new one — but the frame
cache bounds them (a furnished home measured well under the cache over
twenty-minute visits), and deleting images as they leave the cache is
the known fallback if a terminal ever runs short. Rejected: pieces
solid to her with overlap only at fixed seats — each piece cut its
floor into stretches she couldn't cross, which then needed "ducking
behind" links, all to save images the cache already bounds.

## Houseguest has a door in space (2026-09-28)

**Rule:** When no route reaches where she wants to be, or her floor has
no way off, she goes through a door in space. Clambering over dividers
and stepping out at a screen edge are ordinary links that routes use
first. See [design.md](design.md#houseguest).

**Why:** Terrain comes from whatever the user's layout draws, so some
floors are pits: in line art a chat full of text walls its floor in,
and her home can sit where no route reaches. The user asked for all
three: more ways around are more to watch, and the door is the
guarantee — the one behaviour that works on any screen, so she can
never be stuck for good. It lives in her own box (door and doorway are
drawn in her image), so "ignoring what's there" only ever means the
terrain between, never text. The sofa nap was made a lounging choice
in the same change: answering sleepiness, it always lost to the bed.

## Houseguest furniture answers what she does (2026-09-29)

**Rule:** The lamp is dark while she sleeps, the fridge open as she
looks in, the cat home on about half her visits (decided by the visit's
seed) and biting at the end of a petting; a new piece is only set down
where she fits to use it, in it or beside it. See
[design.md](design.md#houseguest).

**Why:** A piece that only sits there is decoration; the fun is in the
room reacting to her. The cat's presence comes from the visit's seed
rather than her random generator so it holds for the whole visit and
leaves every other seeded behaviour exactly as it was. The placement
rule used to cover only the uses *in* a piece; a cat bed set down with
no floor beside it could never be petted, so the rule now covers the
uses beside a piece too. The fridge brings the proposal's *hungry* need
in, answered by a snack, now that something can answer it. Rejected
for now: the window (a wall piece, and props only stand on floors),
and the kotatsu with seasonal rotation (it belongs with the calendar).

## Houseguest shops, and what she buys comes boxed (2026-09-29)

**Rule:** The TV arrives on her second visit; after that the shopping
channel sells her one piece at a time, at most once every three visits,
and it arrives boxed on a later visit for her to unpack. See
[design.md](design.md#houseguest).

**Why:** The user chose the TV first and the channel after, and about
one piece per three visits (with the default one-minute delay, visits
are frequent; the full set takes a dozen or so). A purchase is recorded
when the channel comes on, not when the scene ends — the proposal's
"room mutations commit at scene start" — so a key press mid-advert
loses nothing. Delivery waits for a later visit so buying and receiving
are two moments, not one. "Boxed" is part of the record rather than an
animation, so an interrupted unpacking simply waits for her: the box is
still there next time. Rejected: a box sliding in from the screen edge
(it would cross text on the way); unpacking as a timed animation that
commits at the end (an interruption would leave a piece half-owned).

## Houseguest keeps a local record (2026-09-29)

**Rule:** Her home, visit count and master seed live in one local JSON
record (settings key `houseguest_ledger`), saved on change; an unreadable
record is kept, not overwritten. See [design.md](design.md#houseguest).
*(2026-09-29: the key was `houseguest`, which the arrival delay setting
already used, so after her first visit the settings failed to load; it
moved to `houseguest_ledger` the same day. 2026-10-04: her clock and
what's rare joined the record, and time is saved in batches and at exit;
see [her record: events at once, time in batches, and on
exit](#her-record-events-at-once-time-in-batches-and-on-exit-2026-10-04).)*

**Why:** A home is the point of phase 3, and it has to outlive the
process. Local like the layout sizes: each client has its own Osaka,
and syncing her would make one user's idle screen everyone's business.
Unknown pieces are skipped rather than failing the record, so an older
build reading a newer home loses a piece, not the home. A record it
can't read at all might be a newer build's: overwriting it would lose
someone's room for good, where starting afresh for one session costs
nothing. Seeding each visit from `(master seed, visit number)` makes a
visit reproducible from the record alone (the first visit's seed is
the master seed itself, so seeded tests are unchanged). Rejected: a
table in the synced store (not shared state); saving on exit only (a
crash would lose the delivery that just happened).

## Resident houseguest keeps out of the focused pane (2026-09-30)

**Rule:** With *Resident* on (the default), she stays through playback
and local input; while the client is in use (playing, a held selection,
or input within the idle delay) the focused pane is protected and rains
her out, she leaves it by her door, whatever would take her into the chat is a tenth
as likely, and local input puts back what she moved in the chat. See
[design.md](design.md#houseguest).

**Why:** It was a pity she vanished the moment the video started — the
screen people glance at during an episode went back to being furniture.
But she still mustn't get in the way. The user named the two levers:
the focused pane is where the user is working, whichever it is, so
nothing of hers may be there; and the chat is where people read, so she
mostly stays out of it. Making the focused pane *protected* reuses every
existing guarantee (her body, bubbles, furniture, moved text, and swaps
all already respect protected rectangles) instead of adding a parallel
"hidden pane" rule. The rain, not an instant cut, because what's lost
is only ~3.6 s in a pane the user just arrived at, and the rain is the
visual language she already speaks; it starts at once (no startled
beat) because she isn't leaving. She leaves by her door rather than
falling or being dazed onto the nearest floor: the door is her existing
"nowhere else to be" move, and a door that opens elsewhere reads as her
choice. A tenth rather than never: she's allowed in the chat, just
rarely, which keeps "nothing is ever ruled out". Typing undoes her chat
mischief so anyone can shake it off with a harmless key (arrow-down)
without her leaving. Her body — not the floor under her feet — decides
whether she's in the focused pane: a pane's top border may be the floor
of the pane above, and standing on a protected line is already allowed.

Only while the client is in use, because the chat is focused by
default: protecting it always would keep her out of the chat on every
ordinary idle visit, where nobody is there to be in the way of. The gate
is the visitor's idle gate (playing, a held selection, recent input), so
"in use" means one thing throughout; arriving is left alone (she drops
onto empty space). Rejected: resident only while playing (two modes of
one guest); hiding the focused pane's contents only (she'd walk around
invisible); banning the chat outright.

## Line-art Osaka passes text, and watches chat briefly (2026-09-30)

**Rule:** In line art she may pass in front of single-width text, which
her image derezzes into alien glyphs, but only stays where her box is
blank or lines; wide glyphs stay solid. A chat message makes her watch
the chat for 15 s after the latest one, not a minute. See
[design.md](design.md#houseguest).

**Why:** Found in a real session at 191×44 with a populated client.
"Never cover text" cut every floor under a line of text into pieces:
series titles over the Users border, user names, the chat log over its
bottom border. So she used the door-in-space fallback all the time, and
could never stand on the scrollback accordion, which always has the
log's text right above it. The rule's intent is that text isn't
*obscured so people can't read it*; a second or two while she walks past
is fine. Derezzing rather than blanking keeps it visibly "text, briefly
scrambled by her". It's a per-character cipher, so it looks deliberate
rather than broken. Resting is still kept off text, so nothing stays
unreadable. Wide glyphs stay solid because her image can't cover half of
one. A minute of watching looked like being stuck: a lively chat kept
her frozen for minutes. 15 s is enough to read as "she noticed", and the
4 s startle per message still happens. *(2026-10-06: 5 s; see
[a chat line's watch is 5 s](#a-chat-lines-watch-is-5-s-2026-10-06).)*

## Scrolled-back chat gets an accordion Osaka pokes (2026-09-30)

**Rule:** While the chat log is scrolled back, its bottom border is a
`╱╲` accordion counting the messages that arrived below. A minute after
the first unseen one, and then at most once a minute while more arrive,
it is poked and shakes — by Osaka, arriving for it if need be, or by
itself when visits are off. See [design.md](design.md#chat).

**Why:** A player twice scrolled up during a session and forgot, missing
the conversation below. A static marker isn't enough — people are far
better at noticing motion than patterns — so it moves, and she's the fun
way to make it move. The border, not a row of the log: it costs no
content rows and sits right above where you type. `╱╲` rather than
ASCII `/\`: the box-drawing diagonals join into one zigzag, and reading
them as floor lets her stand on it without special-casing the terrain.
The clock is the first *unseen message*, not the scroll: being scrolled
back costs nothing until something is missed, and it must not reset on
scroll input — people scroll while reading. Once a minute and only with
more unseen, so it's a nudge, not an alarm. Her errand overrides the
idle gate (she comes during playback), the focused-pane rule (the chat
is usually focused), and a visitor's leave-on-input (she finishes the
poke), because the poke is the point; the ordinary rules take over the
moment it's done. The shake is painted by the guest over the frame, not
by the chat pane, so nothing flows back from the overlay into the UI.
Clicking the accordion follows the newest line (there was no key for it:
`End` belongs to the input). Rejected: shaking on a timer from the
scroll itself; a separate "N new" row in the log; a sound or OSD
message.

The first real test showed she could never get there in line art: the
log's text sat over the accordion, and `╱╲` wasn't a line her image
redrew. Now her image redraws the diagonals, and she may stand over the
log's text for the poke (the spot covering least of it). She comes and
goes by door unless she's close by, so she doesn't trail along the text.

## Houseguest makes furniture of torn-off text (2026-10-01)

**Rule:** With no sofa or bed to use, she tears the end off a line
within reach and crumples it into a makeshift one, drawn as shreds of
the derez glyphs in the text's colours; the torn glyphs are holes until
the piece goes. A real piece of its kind wins nineteen times in twenty.
See [design.md](design.md#houseguest).

**Why:** Most clients' Osakas owned no furniture yet (the progression
is slow, and full panes put pieces in the closet), so the room and its
uses rarely showed. She has needs and the screen is full of text: making
do with it is in character, and minor text changes are fine as long as
they go away when their pane is selected — the text layer's validation
and the focused-pane rule already guarantee that. The shreds reuse the
derez's per-letter patterns, so the letters visibly become the piece;
because the piece is her own furnishing, her image may cover and
composite it like a real one (she sleeps *in* the bed, under shreds),
which text in the terminal font could never be. "One time in twenty"
means that when the whim lands she goes for the makeshift one; picking
uniformly between it and the real one would have halved the rate (the
first draft did: 2%).

The text first reels in to her hands and vanishes there, and the piece
is made where she stands (user review, 2026-10-01): she's crumpling it
between her hands, so it shouldn't simply blink out of its line while
the piece appears elsewhere.

**Rejected:** A heap of real text glyphs she perches on top of (hops
up a row; line art can't cover text, so it had to sit under her box,
which needed a new perched state, hops, and a floor that isn't a line)
or beside her as a pillow (less of a bed) — the user pointed to the
derez art instead, which made the piece ordinary furniture. Weighting
the offer down further ("it's a bother", 0.3 then 0.6): with the brain's
top-four cut, she then almost never made one, and the real-piece rule
already carries the discouragement.

## Houseguest reads the frame she stands on (2026-10-01)

**Rule:** Every terrain she's placed on is read from the real frame
with the same solid set (protected cells plus the text she moved and
its holes); her own overlays (the accordion's shake) are painted after
every read; she only stays (a seat, a calm spot, anywhere she rests)
where the whole image she'd be drawn in, with the pieces she overlaps,
is clear of text — one definition of that image, in the terrain, which
the drawing's choice of pieces shares. See
[design.md](design.md#houseguest).

**Why:** Three bugs found by a 256-case run of the property tests,
all one class — a spot chosen against one picture of the screen and
used against another. The accordion errand chose its spot on a terrain
without her moved text, so the spot had no floor in her own terrain and
she bounced between it and a door forever; the errand's poke shook the
accordion before the terrain was read, so the shake moved the end of
her floor out from under her, every poke; and a seat checked only her
box, while the image she and the sofa share spans their bounding
rectangle, which hid text for a whole nap. Moved-text cells also go into
the solid set as row runs rather than single cells: the per-cell scans
were ~40% of a long visit's time. Later runs found a fourth of the
kind: a wide glyph half inside a protected area, whose outer half she
could draw on — and drawing beside half a wide glyph blanks all of it,
protected or not; protected areas now take in such a glyph whole.

Post-mortem (same day): the seat fix was local — `stays_calm` gave
seats the shared image's rectangle, while every other "may she stay
here" (the calm spot she walks to off text, the spots scenes and the
stage pick) still checked her box alone. A 24-case gate run found her
spacing out, then lying down, beside her bed with text in the image's
corner for 17 s. The class is the rules and the renderer each holding
their own idea of what her image covers; the fix is one definition
(`terrain::image`): the terrain carries her furniture's covers in line
art, `restful` checks the whole image, `stays_calm` is gone, and the
drawing picks the pieces that go in her image with the same function.
An adversarial review of the fix found two more of the kind: the image
takes in any piece its rectangle meets, not only those meeting her box
(else a piece drawn apart inside it is overdrawn), and a makeshift
piece she plans is judged with its own cover in her image, at the spot
she'll crumple it as well as where she'll use it.

## Houseguest rechecks where she rests (2026-10-01)

**Rule:** While she rests where she settled calm, uses a piece, or
works at a job's spot (pull, tear, swap, and the giggle after), the
spot is rechecked every frame against the image she's drawn in; when it
stops being calm she's startled, lets go of the job, and moves on. If nowhere was calm when
she chose, nothing is rechecked. See [design.md](design.md#houseguest).

**Why:** "Never stays over text" was checked only when she chose, so
text arriving under her mid-rest, or a piece of her furniture
reappearing beside her (the text hiding its spot went back) and growing
her image, stayed hidden for the rest of the act — up to 40 s lying
down. Seats already had the per-frame check; the user chose to make it
the invariant for every rest. The recheck is limited to spots that were
calm at the choice, so a terminal with nowhere calm doesn't startle her
over and over. The first cut left work on text alone, and the property
test found her walking to a swap spot as text arrived there, then
swapping, giggling and whistling over it for 10 s: job spots are chosen
calm like rests, and letting go mid-job was already safe (a swap's undo
is scheduled when it's made; a dropped tear's glyphs go back). The
moments after a sneeze, a fall or a door are short and stay passing.
The test models the trigger honestly — text coming up next to her at
random times — and failed before the change.

## Two body bugs from the deep runs (2026-10-02)

**Rule:** Somewhere calm elsewhere is chosen among the calm spots of
each floor, so she never stays over text while a calm spot exists.
What she painted (which the goodbye and a focused pane's rain fall
over) is only what her image could cover. See
[design.md](design.md#houseguest).

**Why:** Both turned up in 256-case runs during phase 0 of the
[mind proposal](proposals/2026-10-02-houseguest-mind.md). Off text with
no calm spot on her own floor, she tried one random spot per floor and,
when every one was on text, concluded there was nowhere calm and stayed,
hiding text behind her for over ten seconds (a floor with 67 calm spots
out of 77 still missed one time in eight). And when a pane was focused
as she stood against its border, the next frame rained noise over the
border: her image is clipped there and never drew on it, but her frozen
composite listed every cell of her box.

## Her sofa goes with her TV (2026-10-02)

**Rule:** Pieces of a room whose places along the floor collide stand
side by side in order, while the pane holds them all. A sofa on the
TV's floor (the floor she'd stand on to watch it) is where she watches
from. A makeshift sofa is five times as likely to be made where it
would face the TV. See [design.md](design.md#houseguest).

**Why:** The user saw her TV on one floor and her sofa on another. The
watch-from-the-sofa check asked for the floor under the TV's leftmost
column, which against a pane's wall is no standing spot, so a TV in a
corner was never watched from the sofa. Two pieces at colliding places
sent their whole room to another pane or the closet, though they'd fit
side by side. And a makeshift sofa was built under whichever line she
picked, uniformly, wherever the TV was. These are the phase-0 patches;
the [proposal](proposals/2026-10-02-houseguest-mind.md#home-and-structures)
replaces fixed places with anchors, rooms from contents, and rules
she repairs.

## What she makes a piece for stays on the piece (2026-10-02)

**Rule:** A makeshift piece records what she made it for and whether
she has started using it. Once the reflexes (an errand, watching the
chat) let her, she goes back to a piece she hasn't finished with before
choosing anything new, the nearest first, until she uses it; each
setting-off, and each time she can't get to it, is a try, and after
three without progress she lets it be. See
[design.md](design.md#houseguest) and the
[proposal](proposals/2026-10-02-houseguest-mind.md#interruption-and-intent).

**Why:** The user watched her crumple text into a sofa and never sit on
it. Her plan lived in one slot in her head (`making`): a second build
overwrote it, an interrupted crumple left it stale, and a crumple
finished later found no plan (16 of 16 cued runs; the
[diagnosis](proposals/2026-10-02-houseguest-mind/sofa-diagnosis.md)).
Purposes kept in the world (a parcel's box, a heap's stage) had always
survived interruption; so the purpose moved onto the piece, and nothing
she does can lose it. Pieces are told apart by an id, not by recomputing
a seat on both sides. The tries keep it from becoming a chore list: a
heap whose seat stays blocked, or chat that keeps interrupting, doesn't
hold her forever. "Nearest first" because the property test found her
walking away from the heap under her feet, to a sofa on another floor
(and losing tries to it). A walk to a made piece on another floor is
dropped when chat interrupts it, so coming back is a counted try; kept
as a plain goal, it was resumed uncounted, and a chat line every 20 s
caught her on the same tall divider again and again.

That loop also exposed a body bug: a chat line while she clambered
over a divider set her looking in mid-air, so she fell and lay dazed.
Climbing and falling already finished first; clambering is now among
them, through one `aloft` test that the look, her errand and her line
art's floor all use. And what she does to her home (an order off the
shopping channel, an unpacking) is recorded right after the tick that
did it: it waited for the next paint, so a visit ending in between
(a visitor's key press) lost it.

**Rejected:** Re-validating a plan by struct equality against a fresh
frame (it silently dropped jobs whenever chat scrolled); a planner or
an intention stack in her head (the proposal's
[alternatives](proposals/2026-10-02-houseguest-mind.md#alternatives-considered)).

## A made piece is for what the real one is; a nap is a doze (2026-10-02)

**Rule:** A makeshift piece, once in shape, offers the real piece's
uses (the sofa: sitting, napping, and TV when it's on the TV's floor).
A nap answers *sleepy* (0.1 a nap), but never fits below an offer that
answers no need. See [design.md](design.md#houseguest).

**Why:** The user watched her make a sofa, then lie down to sleep on
a border of the List pane for the rest of the visit. A makeshift sofa
offered only sitting, which answered no need, while a doze on a border
answered *sleepy*, so a sleepy Osaka always preferred the floor to a
sofa she'd built. With the nap offered and answering *sleepy* she
naps on her sofa about three times as often as she lies on the floor
(stage room, kept sleepy). A nap scored by sleepiness alone vanished
from furnished homes (8 naps against HEAD's 30 in 8 × 30 minutes),
since the bed wins whenever she's sleepy; the neutral floor keeps the
casual nap. At 0.2 a nap those casual naps kept her from ever getting
sleepy enough for bed (10 bedtimes against 18); at 0.1 it's 15. The
proposal's needs model (comfort, spot quality) replaces this bridge
([proposal](proposals/2026-10-02-houseguest-mind.md#every-want-answers-a-need)).

## Her intent lives in her act (2026-10-02)

**Rule:** The job she's at, or walking to, is part of her act (Walk
carries what she does on arriving; each job's act carries its job), so
it can't outlive the act or be missing from it. Every act is classified
once (`ActProps`: where she stays, what a chat line does), and the
queries that used to keep their own lists of acts read that. One
`interrupt(Cause)` is where anything stops her short. This is
implementation, not behaviour: [design.md](design.md#houseguest) is
unchanged.

**Why:** Phase 1 of the
[mind proposal](proposals/2026-10-02-houseguest-mind.md#the-body-interface).
Her plan was split across the act and a separate `task`, which some
fifteen sites cleared by hand, and about nineteen `match` sites listed
acts by kind; `recheck`'s first list was wrong (654dd11) and the sofa bugs were
stranded intent of the same kind. Golden trajectory hashes (32 runs,
recorded before the change) prove it changed nothing she does. The one
reader of `task` that didn't go through its act was `lost_grip`; it
keeps the broad check (any job in her act, a walk's included), because
a tick far behind can re-pick a pull before the paint that reports the
lost grip.

## Her mind is a table and a heading (2026-10-02)

**Rule:** What she wants is a table (base, needs answered, factors).
How she sets about a want is a list of named methods with pure guards;
a want is offered only when one of them binds, and she does what it
bound. Her mind draws from its own random stream, once a decision.
Heading for another floor, she finds the job again by meaning and
weighs it three times as likely each time she chooses; choosing
otherwise, or finding it gone, lets it go. See
[design.md](design.md#houseguest).

**Why:** Phase 2 of the
[mind proposal](proposals/2026-10-02-houseguest-mind.md#choosing-what-and-how).
Offers and plans were built by separate code (`decide`'s offer list,
`start`'s arms, `places_for`), so a want could be offered and then turn
out impossible ("couldn't after all", re-rolled), and adding a
behaviour touched four to six places. With the guards also planning,
the two can't disagree, and a behaviour is a row plus a method or
three. Guards read whims hashed from one draw a decision, so they stay
pure and the body's stream (durations, looks) no longer shifts with
every choice. The heading replaces a goal that was found again by
exact equality, so a line that scrolled one row was a different job
(83 and 125 dropped pulls in two long runs with scrolling chat, in
the [diagnosis](proposals/2026-10-02-houseguest-mind/sofa-diagnosis.md)).
It was also a certain pre-empt; now, after an interruption, it competes
with inertia, as the proposal's rule 3 has it, so a long trip can be
abandoned for something more pressing. The first cut rolled again at
every hop, and the visit simulator caught it: in a furnished home 22 of
135 trips arrived (rule 1 says a landed hop is the next step of what
just completed, not a choice); with hops carrying on, 103 of 128 do,
the rest let go after chat. A heading toward a piece she made stays certain: that
purpose is on the piece, and continuation already counts her tries at
it, which a roll per hop would double.

Needs are eased by the share she does of what she chose (Q4): an
interrupted sleep took off all 0.7 of sleepiness at the choice, so she
woke from five seconds of sleep as rested as from three minutes and
didn't go back to bed. Zero credit for an interrupted act was rejected:
it would bring back the doze oscillation (2026-09-28). Walking and
travel stay credited at the choice, since setting off is already the
moving.

Every want answers a need (the user's Q2), so her furniture is worth
something to her: comfort, fun and daydreams are new, and the spot she'd
use counts (a real piece fully, one she made 0.6, the floor 0.1 for
comfort; 1, 0.7, 0.3 for sleep), which makes a sofa beat the floor
without a special case. The fit weighs each need by how much the want
answers it (× 2, so an answer of 0.5 weighs what a whole need did
before): with every need counted fully, wants answering two needs won
too easily (homework, which answers daydreams and comfort, rose from 5%
to 15% of her choices at home in the simulator). Fun wears thin per
source (RimWorld's joy tolerance), or the TV would answer it every time.
Sneezing stays an accident with no need: mischief sits at 1 all visit in
a room with nothing to swap, and would have her sneeze every other
choice. A parcel and her job keep a fit of at least 0.5, as a nap did.
The new needs start at 0.5, so wants that scored a flat × 0.5 aren't
cut to × 0.1 at arrival. Beauty and nesting wait for phase 4.

Her mood for the visit (the user's Q10: "She has a life outside
dessplay") is one row of need-rate multipliers, not new selection
logic, drawn from the visit's seed so neither random stream shifts. Its
greeting makes a lazy visit read as her choice rather than a broken
feature. Tuned in the simulator (30-minute visits, 16 seeds, forced
moods): for a resident, lazy spends 42% of her time on furniture and
26% moving, industrious 24% and 41%; dreamy spaces out 10% against 5%.
An industrious visit's faster tidying barely shows where text keeps
tidy pinned high anyway; its home acts come with phase 4.

Every loss owes a beat (a glance, sometimes a line) for the
proposal's rule 5, "nothing ends silently": a lapse reads as hers when
she looks at what she lost (Carlisle's gaze), and a dropped purpose
that shows nothing reads as a bug. Beats wait behind the reflexes and
stay owed if interrupted, so they're never lost themselves; lines have
a cooldown and a budget because about a third of tears are abandoned
under chat, and a line every time would nag.

The method table is not a behaviour tree or a planner: methods hold no
state, a decision makes one step, and the only memory is the heading
and what's on her things. Trees were rejected (2026-09-28) for intent
living in a running node that every interrupt must unwind; GOAP for
planning against a hypothetical terrain, which would move the
read-order bug class rather than remove it, for a catalogue of which
about 70 of 90 entries gain nothing from search.

## Her home stands on strips, and its rooms are what's in them (2026-10-02)

**Rule:** Each piece stands on a strip (a quiet pane's bottom border)
at an anchor of its own: a wall side and an offset. A strip's pieces
are packed in anchor order, an order that doesn't depend on the
strip's width. A strip that's gone or too small moves its pieces,
together and in order, into the first strip that holds them with its
own. A room is a strip, and its role comes from what's unboxed there,
by a table. Parcels come in through a flap at the screen's edge. She
watches from a sofa that faces the TV on its strip. The record keeps
version 1 and adds the anchors as a new field. See
[design.md](design.md#houseguest).

**Why:** Phase 3 of the
[mind proposal](proposals/2026-10-02-houseguest-mind.md#home-and-structures).
A piece placed by its share of a pane's floor drifted with every
resize. Live platforms can't be a home's floor either: text splits
and renumbers them every frame. Anchors from a wall keep a fridge
against its wall, which phase 4's rules measure in cells. The packing
order runs left anchors, then right anchors, so it never depends on
the width: a resize can't reorder pieces, and a resize and back
restores them. Ties put the newer piece nearer its wall, since it came
in through that wall's flap.

Rooms from contents was the user's answer to Q7: panes are only her
starting home. That deletes `RoomKind`, "one room per pane" and room
binding; an older build still needs a pane per room kind, so the
ledger keeps a private copy of the old kinds to write `rooms`.

The user's 2026-09-28 rule that a room losing its pane "moves whole"
stays, now per strip: the pieces move together. Any strip that holds
them will do, not only a free pane.

**Deliveries:** with no room binding a parcel, where should it land
before phase 4 can tidy up? The user's call: at the edge of the screen,
through a flap, "it's a package, after all". Rooms may mix until phase
4, which lands before any of this is pushed. The pieces on that strip
make way only where every one that shows still fits, so a delivery
never closets furniture.

**Faces** (the user's call, out of three): same strip and a 2–14 cell
gap now. Which way the sofa faces counts from phase 4, when she can turn
it, so no sofa loses watching before she could fix it. Judging on the
strip fixes the split-floor case (a protected run or a wide glyph
between sofa and TV). A sofa pushed flush against the TV (gap 0), as a
second parcel through one flap can be, isn't for watching until phase
4 moves it. Made pieces stand on no strip and keep the same-floor rule.

Rejected:
- **Re-anchoring a piece that merely doesn't fit, rather than one whose
  strip can't hold the set.** Text over a piece closets that piece; it
  must not send the room wandering (2026-09-28).
- **A geometry-dependent order** (by where each anchor lands this
  frame). It reorders pieces on a resize.

## She watches from a sofa turned toward the TV (2026-10-03)

**Rule:** A sofa faces the TV only when it is turned toward it, as well
as on its strip and 2–14 cells away; watching from it, she sits the way
the sofa is turned. The TV's own way round doesn't count. A makeshift
sofa keeps the same-floor rule and turns her toward the TV. See
[design.md](design.md#houseguest).

**Why:** the [2026-10-02 Faces call](#her-home-stands-on-strips-and-its-rooms-are-whats-in-them-2026-10-02)
left the sofa's way round out until she could turn it, so no sofa
would lose watching before she could fix it. Phase 4 brings that fix
(the Faces rule and its repair, a turn in place being the cheapest),
and it lands as one push, so the promise holds. Watching from a sofa
turned away was also a picture that made no sense: she sat facing the
wall with the TV behind her, while the watch seat turned her round in
place. The seat now takes the sofa's own facing, which is the TV's
direction whenever Faces holds, instead of a second derivation from
the two pieces' columns. A makeshift sofa stands on no strip and has
no way round of its own, so it keeps that derivation. The TV is seen
from the front from either side; turning it would only change its
drawing.

## She feels what's wrong with her home, once a visit (2026-10-03)

**Rule:** The rules of her home are judged on her pieces' layout, not
on what shows. Using a real piece a broken rule involves, for a use the
rule is felt on, she says so for two frames, and has felt it once that
has all shown. Each rule is felt at most once a visit, outside her beat
lines' budget, and forgotten with the visit. Nesting rises only while a
felt rule is broken. Home acts are capped by mood: lazy 0, ordinary 1,
dreamy 1, industrious 3. See [design.md](design.md#houseguest).

**Why:** *Once a visit, forgotten with it* is the user's call
(2026-10-02, confirmed 2026-10-03). She grumbles about a wrong thing
the first time she runs into it on a visit, which shows the player
what's wrong without nagging, and grumbles again on the next visit if
it's still wrong. Nothing is persisted: the ledger records her home,
not her moods about it. A felt rule is what lets nesting rise (and
later lets her arrange): she minds what she has noticed, not what an
omniscient check knows.

*Felt only once it has all shown*: feeling is what she said, so a line
cut short (a chat message, a startle, the use ending) doesn't count.
She feels it again on her next use of the piece. The window waits for
anything she was already saying, and nothing she says later covers it,
so an uninterrupted use always says all of it. It is marked on the
window's time passing while she's still on the piece, not on the
bubble having been drawn: a line that found no room on a crowded
screen, or that she said hidden behind text, still counts. She said it;
tying felt to the paint would make what she minds depend on where the
text happened to be, and the player sees the consequence (her setting
about it) either way.

*Outside the beat lines' budget*: those lines are rationed (a budget
and a cooldown) because a beat can recur all visit. A grievance can't:
each rule is felt once a visit by construction. Rationing it would drop
some silently, and a dropped grievance would never be felt, so nesting
would never rise for a home that is plainly wrong.

*Over the shopping channel's advert*: one bubble at a time, and an
advert that comes on while she isn't looking would be a purchase the
player never saw her make. The channel sells again on a later watch.

*Judged on the layout*: text closets a piece frame by frame. Judged on
what shows, a chat line over the sofa would make the room right or
wrong at random. She would feel, and later mend, what the chat did
rather than her home. The layout is hers: where the anchors put the
pieces on the strips, closeted or not.

*Caps by mood* are the user's call: a lazy visit leaves the home alone,
and an industrious one sets several things right.

## She puts her home right, with the piece in her pocket (2026-10-03)

**Rule:** A rule she has felt is mended by moving one of its pieces,
the cheapest way that holds this frame, worked out at paint. She lifts
the piece into her pocket (shown nowhere, its place kept), carries it
and sets it down; the next paint takes it if the move still holds and
fits, else she lets it go and it's back where it stood. The piece is
hers until then, through every interruption; she doesn't choose anything
new meanwhile. See [design.md](design.md#houseguest).

**Why:** *The pocket over a lug* is the user's call (proposal Q9,
2026-10-02): a carrying animation for every piece and pose costs far
more than a magic pocket, and the pocket is in character. A visible lug
for short moves stays a possible gag once the image budget is measured.

*Worked out at paint, not as she decides* (a deviation from the
proposal, which ran the search in her decision with a snapshot of the
frame's blank cells): whether a piece fits is one definition, the one
her pieces are shown by ([one definition of where she
stays](#houseguest-reads-the-frame-she-stands-on-2026-10-01)). A second,
snapshotted copy of it would drift from the first, and a move judged on
one would be refused by the other. The paint already has the frame, so
the search runs there, throttled, and her decision reads its result.
For the same reason the set-down is committed at paint, judged again
with the piece moved; a goodbye before that paint loses nothing of her
record.

*Unsettled pieces move first*: she never chose where a delivery stands,
so moving it is putting it where it goes; moving a piece she has
settled undoes a choice she made. A turn where it stands is cheapest
of all, whoever chose it. The ranking is by tier, then by cells moved:
a delivery goes first to a room it completes (the TV joining the sofa
makes a living room of a den), else one it doesn't spoil, else an
empty strip; where the rule would move an unsettled piece too, moving
the settled one comes after all of those. Tiers before cells because a
far room the piece belongs in beats a near one it merely fits: by
cells alone, the TV would go to the nearest room it doesn't spoil
rather than to the sofa it's for.
These are the role tiers proposed for phase 3 and set aside for the
flap (phase 4's brief brought them back). A delivery that would spoil
a room isn't a way at all: it would only break `Belongs` again.

*The carry is certain*: a half-done move is a piece missing from her
home, so she never rolls away from it as she would from a heading.
What ends it is setting it down, the frame refusing it, or her tries
running out, and then she glances back at it where it stands again.
While a step waits for the frame (just lifted, or set down) she waits
too, rather than read an old frame's judgement as a refusal.

*Tries count failures, not settings-off*: the piece is hers through
every interruption, so a chat line, a startle, an errand or her door
must not cost her one; otherwise a busy chat (three lines during one
carry) made her drop a piece she was carrying fine (review of the first
cut, 2026-10-03). A try is a setting-off that comes to nothing (no way
there, or her walk ending short of it, which the walk itself reports),
so a blocked step can't loop forever, and interruptions, which come
from outside her, can't use her tries up. Unreachable, she waits a
couple of seconds between tries, so a spot that's busy for a moment
doesn't cost her the move.

*One fix per felt rule, capped by mood*: she mends what she noticed,
once, and only as much as her mood allows (lazy, nothing). A satisfied
rule never moves anything, so she never rearranges a home that's right.
One go means one: a move she lets go of stays let go that visit, else
the same move, refused again for the same reason, would play "Hup!" …
"Oh well..." over and over, and the rule would keep her keen on a home
she can't put right.

*A rule no move mends doesn't block the next* (the phase-4 census,
2026-10-03): the search first ran only for the first rule she felt that
was still broken. When no single move mended that one (the sofa and TV
in two rooms where joining them would change a room's role, say), she
never got to the bookshelf she felt next, which one move would have
put right: in the census's furnished home, 3 of 16 industrious visits
did nothing about their home all half hour, and 11 ended with a rule
felt and broken though her cap left her more to do. Now the frame
works out each felt rule in the order she felt it and keeps the first
a move mends; while she's moving a piece, only that piece's rule.
After the fix every ordinary and dreamy visit there makes its one move,
and industrious ones make one or two. A rule no move mends still keeps
nesting high for the visit; nothing comes of it, since no way to
arrange is on offer.

## She tries a piece in a spot or two (2026-10-03)

**Rule:** When a few spots (up to three, the same piece, the same tier,
within 4 cells of the cheapest; one per spot) are as good as each other,
she tries the piece in them in a whim's order: set down and taken, she
sits on it a moment ("hmm..."), then keeps it with probability
e^(−Δ/T) (Δ how much dearer than the cheapest, in 4-cell units; T her
restlessness, at least 0.05) or lifts it again for the next spot that
still holds; the last she keeps. One home act, however many spots. See
[design.md](design.md#houseguest).

**Why:** *Trials are behaviour, not search.* A person putting a sofa
right doesn't compute the optimum; she shoves it, sits, frowns, shoves
it again. The search already finds the cheapest way, so the trial only
ever settles between ways that are nearly as good, and it costs nothing
in correctness: every spot tried puts the rule right and breaks none
(each is re-checked, from where the piece stands, on the frame of the
moment). It's the only annealing in her home: everywhere else she takes
the cheapest way, so the home a visit leaves is still the one the rules
say, give or take a few cells.

*The keep probability*: e^(−Δ/T) keeps the cheapest spot always (Δ 0),
so trials end where she would have gone anyway unless she happens on a
dearer spot first. T is her restlessness: keen to be up and about, she
doesn't fuss over a cell or two; calm, she's picky. Δ is capped at 1 by
the 4-cell window, so even a calm Osaka doesn't fuss over a far worse
spot (there are none in the window), and the last spot is kept outright
so trials always end. With whim order and restlessness near its usual
high, she tries a second spot about one move in seven where three
spots are as good (measured on the sofa turned from the TV), a third
rarely.

*Whim order, mind stream*: the order and the keep roll are drawn from
her mind's stream (salted by the spot's place in the order), never the
body's, so trials don't shift her durations or looks, and decisions
stay replayable.

*One home act*: the trials are one change of mind about one rule, so
they count once against her mood's cap (at the first set-down, when the
rule first holds); a lifted-again piece is carried past the cap. Being
let go of mid-trial (refused, tries spent, a goodbye) leaves the piece
where she last set it down, which already mends the rule.

*A turn is no other spot*: two ways that differ only by which way the
piece faces would be a trial no one could see, so each spot appears
once, the cheaper way round. (Found by `no_piece_is_moved_twice`: the
lamp was lifted to be turned where it stood.) And the search turns a
piece only where its rule looks at which way it faces (the sofa, to
face the TV): turning a lamp, a fridge or a TV mends nothing, and its
turned copies (+1 each) crowded the three ways kept with the same
spots, so a lamp had fewer real spots to try (2026-10-03, review).

*A spot with nothing left to try is kept*: when every other spot she
meant to try fails its re-check, the piece stays where she last set it
down, which already mends the rule; she says "There!" as for a spot she
chose to keep, rather than going quiet after "hmm..." (an episode that
ended without a word looked like she'd forgotten it).

*A trial sit is a moment of a use*: she's eased by the share of a whole
use of the piece she did (a few seconds of a lounge is a little of
one), as for any use cut short; crediting it as a whole use would make
trying a sofa as restful as lounging on it.

*The image budget held*: trials re-show the piece up to three times;
measured before they shipped (plan.md, Phase 38), the carry costs some
30–40 distinct images and the cache didn't thrash.

## Pieces hang on the wall (2026-10-03)

**Rule:** A piece that hangs (the poster) hangs 4 rows above its
strip's floor, in a wall lane of its own: packed in anchor order among
hung pieces only, over any piece that stands. Only the floor lane moves
a room; a hung piece its wall doesn't hold is in the closet alone, and
goes along when the floor moves. A crowded wall hangs, in anchor order,
each piece that fits beside those before it; the rest are in the closet.
A strip that's gone with only hung pieces moves them to the first other
wall that holds them all on free cells, else they're in the closet.
Boxed, it's a parcel on the floor, and a delivery checks both. See
[design.md](design.md#houseguest).

**Why:** *The user's call*, reversing 2026-09-29's "props only stand on
floors" (the window was rejected for it): a poster above the sofa is
what a lived-in room looks like, and the floors are crowded enough
without decor taking a slot of them.

*A lane of its own*: hung pieces overlap standing ones' columns by
design, so packing them with the floor would push the sofa along for a
poster. Packing each lane on its own keeps every floor rule (anchor
order, a resize and back restores) for both, with no new layout.

*Hang ≥ the tallest standing piece*: at 4 rows (the TV, bookshelf,
fridge and lamp are 4 tall, and so is she) a hung piece never overlaps
a standing one, her box or her image as she passes under it, so none
of the overlap rules (one image, covers, her rest) need a wall case. A
lint asserts it against the table. 4, not 5: the poster then needs 6
clear rows, and the home screen's Users pane has 7.

*The wall never moves a room*: a strip moves its pieces only when its
floor fails; letting a poster that doesn't fit the wall's height move
the sofa and TV would trade a room for a picture. The hung piece is in
the closet alone, as text over it would leave it.

*Crowded, each that fits*: the first cut packed the wall lane all or
nothing, so one poster too many closeted every picture on the wall.
Keeping, in anchor order, each piece that fits beside those before it
closets only what the wall can't hold, as A1 says, and stays
deterministic (review 2026-10-03).

*A gone wall's pictures move only where they show*: the first cut
moved a strip's hung-only pieces to the first other strip whatever its
wall could hold (its floor test was vacuous with no standing pieces),
which only traded one closet for another while a wall that could show
them went unused. They move to the first wall that holds them all on
free cells, else stay in the closet (review 2026-10-03).

*Delivery checks both states*: the parcel stands on the floor until
she unpacks it, then hangs at the same anchor; checking only one would
deliver a poster that can't be unpacked, or one that can't hang.

## A bare room wants decor (2026-10-03)

**Rule:** A potted plant and a poster (beauty 1 each, offering only
Decor, which no room rule names) come after the furniture on the
shopping channel, and first while *beauty* is her most pressing need.
Beauty arrives at 0.3, rises over twenty minutes only while she's in a
plain room (the strip she stands on shows nothing pretty, or she's on
none), and is eased passively: resting or using her things in a pretty
room, by the share done times the room's beauty (at most 1); exercise
(jumping jacks, touching her toes, stretching) and chores (unpacking a
parcel, crumpling text) don't. See
[design.md](design.md#houseguest).

**Why:** *Decor answers the room, not an act*: there's nothing to do
with a poster, so no want answers beauty and it isn't scored; it only
steers what she buys, and is eased by living in a pretty room (sitting,
sleeping, reading there), which is how a room feeling nice works. Rest,
not exercise: jumping jacks in a pretty room is no more taking it in
than in a bare one (the first cut eased it on any idle act). Nor is
bending over a box to unpack it, or crumpling torn-off text: a parcel
isn't one of her things yet, and both are chores (review 2026-10-03).
*Most pressing, ties counting*: needs clamp at 1, where restlessness
often sits; a strict "highest" would let a saturated restless need
block decor forever. A need at 0 is never pressing. *Then furniture,
then decor*: decor never displaces the next piece of furniture unless
the room's bareness is what bothers her most, so a home still fills up
in the order the user chose, and decor still comes once there's no
furniture left. *Older builds*: decor claims no room's pane in the
record (a poster first on a strip would otherwise take the living
room's pane from the sofa), and their reader skips it as an unknown
piece. *Arriving 0.3*: a little bothered, so a long visit in a plain
room can make it pressing, while a short one rarely does.

## Her image cache drops what she showed longest ago (2026-10-03)

**Rule:** Each distinct image of her (with the pieces drawn in it) is
transmitted once and cached, up to 1024; a full cache drops the image
shown longest ago, one at a time. See [design.md](design.md#houseguest).

**Why:** the cache used to start over when it filled, so the next
frames encoded again every pose she was in the middle of using. A
long, busy visit fills it however it's spent: she walks and travels
over text, and every spot she passes is a new image (some 210–265
distinct images in 20 industrious minutes in the furnished home). In
the phase-4 measurement one such visit encoded 385 images, 120 of them
ones it had already had, though the carry it was measuring cost only
15–25. Dropping the one shown longest ago keeps her working set (the
poses and pieces around her now): at the 256 then in force, that
visit encoded 265, dropped 9 stale images and encoded none twice. A `BTreeMap` keyed by when each
was last shown picks it, so the choice never depends on hash order.

*Sized for a long visit (the user's call, 2026-10-03, having seen
Ghostty hold far more images than she used).* At 256 the cache still
filled within 20 minutes on busy visits (industrious in the furnished
home with a fridge: seed 2 dropped 111 and encoded 19 again, seed 0
dropped 16), and every image encoded again is sent again and stored
again in the terminal. The image census (`image_census` in the
houseguest's tests/census.rs: every mood forced, 16 two-hour visits
each, a cache that never drops, at 10×20-pixel cells) measured two
kinds of screen. On a still one (the stage, her furnished home and
the resident's room, at 100×20 and 100×30 cells) the most images any
visit showed was 421 by 20 minutes, 591 by an hour and 664 by two
hours, and the largest working set (the smallest cache that would drop
nothing she shows again) 603. Growth slows as a visit goes on (her
poses and pieces repeat; what's new is mostly where she passes over
text), to one or two a minute. In a live chat (200×50, a line every 45
seconds scrolling the rest up) it doesn't slow: each line puts new
text under her, some ten new images a minute for as long as the chat
runs, so the working set grows with the visit (1663 by two hours) and
isn't the measure there; what is, is how many a cache would encode
again. The census counts that for 256, 512, 1024 and 2048 from each
showing's reuse distance (the images shown since it last was).

1024 keeps every image of every still visit, with some 70% to spare
(none encoded again where 256 encoded up to 500 again in a two-hour
stage visit, and 512 up to 5), so the busy-visit tests now hold her to
no image dropped and none encoded twice; past it, the stalest goes
first as before. In the live chat it encodes nothing again for the
first hour, and by two hours at most 196 again in a visit, where the
busiest showed 1669 new ones: the terminal's store grows with the new
ones, which no cache can spare it, and the re-encodes add about a
tenth. The cost is client memory: ratatui-image keeps each image's
kitty transmission (raw RGBA in base64) for its whole life, 30–43 KB
an image by room at 10×20-pixel cells (33 KB across the census), so a
full cache is some 30 to 43 MB (the still two-hour visits held at most
22 MB). The limit counts images, not bytes, so that grows with the
cell's area: perhaps four times as much if a terminal reports cells in
device pixels on a 2× display (unmeasured), where a full cache would
approach the client's whole footprint (some 150 MiB, see
[the memory profile](memory-profile-2026-09-19.md)). The terminal stores the raw
RGBA of what it was sent (three quarters of the transmission) until
its own limit evicts it: Ghostty 1.3's `image-storage-limit` defaults
to 320 MB a screen (from `ghostty +show-config --default --docs`),
some 10,000 to 14,000 of her images by room, which a live chat would
take most of a day to fill, so the client, not the terminal, is the
limit.

Rejected: keeping 256, which drops images she shows again within 20
minutes; 2048, which holds a two-hour live chat with nothing encoded
again but then does as 1024 does an hour later (a bigger cache only
moves the cliff), for twice the memory (60–87 MB full, close to half
the client's footprint); an unlimited cache, which a live chat grows
without bound; a budget in bytes (summing each transmission's size)
rather than images, which wouldn't grow with the cell's area: worth
having if a HiDPI terminal proves the image count too loose, not
before it's measured; and deleting an image from the terminal (`a=d`)
as the cache drops it, since ratatui-image picks each image's id at
random and keeps it private (deleting would mean our own kitty
encoder), and on a still screen nothing is dropped to delete.

## A room is spoilt only by what it forbids (2026-10-03)

**Rule:** A piece spoils a room when what the room is without it
forbids something it brings (the rooms table's "no" columns: a bed in
a living room or a kitchen, a TV or a fridge in a bedroom). `Belongs`
and the repair search's tiers use that. A move may change what a room
is, but not make a den of a room that was something; and it may leave
no piece spoiling a room it didn't: the piece it moves the room it
comes into, nor another piece its own room as that one comes or goes
(nor the piece, one she hasn't settled, the room it's set down in).
See [design.md](design.md#houseguest).

**Why:** the user's call (2026-10-03), after the phase-4 census. The
first cut counted any change of role as a room made worse, and a piece
as spoiling a room whenever the room was something without it and
something else with it. That was stricter than any home: a sofa
couldn't join a TV that stands with a desk (the study would become a
living room), nor a bed a desk (a study, a bedroom), though a living
room or a bedroom with a desk is an ordinary home. In the census's
furnished home 11 of 16 industrious visits ended with a rule felt and
broken that her cap left her free to mend; now 3 do (all the sofa not
facing the TV). The room rules don't refuse the move that would mend
any of them; what does hasn't been looked into.

*The table, not the order*: the old test followed the table's order,
so it was lopsided. A desk she hadn't settled in a kitchen spoilt it
(a fridge and a desk read as a study, the row above), while a fridge
in a study didn't. What a room *forbids* is the table's judgement of
what doesn't belong there; which row it happens to meet first is not.

*Only a den is worse*: a den is a room that is nothing in particular,
so turning a room into one undoes it; a room becoming another room is
just the home changing. *No piece may come to spoil its room*, settled
or not, whichever piece moves: the den test alone doesn't cover it (a
fridge joining a bed and a desk makes a study, not a den, but a
bedroom with a fridge in it), and the same room comes of a bed joining
a fridge and a desk, or of a TV leaving a bed, a fridge and a desk.
So the search compares every piece's spoiling before and after the
move, not only the moved piece's (a first cut checked only that one,
and let the other two through). Whether a piece spoils a room hangs
only on what's in it, so a piece shifting inside its own room changes
nothing: a settled piece already there may still move along it (a
fridge to its wall in a room it spoils).

## A chat line over text doesn't stop her there (2026-10-03)

**Rule:** A chat line arriving while her image hides text interrupts
what she was at, but she doesn't stop to look there: she goes on to
somewhere calm first and watches the chat from there. Elsewhere she
stops and looks as before. See [design.md](design.md#houseguest).

**Why:** The 4 s look at each chat line counted as "passing" text,
like walking, so nothing bounded a chain of them. Two lines about 4.5 s
apart, while she was walking across the chat log, kept the same text
derezzed behind her for over 10 s: a look, two steps, a second look.
A lively chat could keep her there for as long as it went on, and the
text she hid was most likely the chat she was looking at. This is the
same problem the 15 s watch fixed
([2026-09-30](#line-art-osaka-passes-text-and-watches-chat-briefly-2026-09-30)),
"frozen by a lively chat", but over text. A shorter look per line
(just the startle) still isn't bounded. A walk step lands only after
its full stride, so lines closer together than that would keep
restarting her. So over text the chat makes her choose at once. That
takes her off the text first, the same as any choice made over text,
and she watches from the calm spot. The trip she was on still ends, as
the chat rule says. She still stops briefly over text when text comes
up under her, because that is what sends her on.

## Vignettes are scripts on the act that hosts them (2026-10-03)

**Rule:** A vignette is a script (a run of keys: pose, face, what she
says, what shows on her furniture) played on the act that hosts it: a
use of a piece, or spacing out. Every use's look is its own script; a
key ends at a fixed time or at a share of the use's drawn length,
counted from the start of the part playing (a prelude, the use, a coda).
She wakes as each key ends as well as on the use's frame grid, which is
kept, anchored at the playing part's start. What a script plays (its
branch, any splice, the lines it draws) is chosen when its act starts,
hashed from the whims of her latest decision, each choice under its own
label. See [design.md](design.md#houseguest).

**Why:** *Hosted, no free-standing act yet*: every phase-5a vignette
happens at a piece or while spacing out, and a use is already right at
the dozen places that ask what she's doing there (where she sits,
losing her seat, how much of it she has done, the census's groups). A
free-standing `Act::Script` would have had to learn each of them for
nothing 5a needs. 5b decides whether the calendar or the dash-ins need
one.

*Every use's look a script*: one player instead of a hand-kept match
per use beside a second mechanism for vignettes. Spans are *cumulative
shares* of the drawn length (a key ends at length × n/d), not
durations summed: summing rounds differently (a watch of 20,004 ms
would have switched at 12,001 instead of 12,002), and shares draw
nothing new. That made the conversion provable: every golden
trajectory and the seed-7 snapshot came through it unchanged, checked
against the old look as an oracle at every grid time and key end.
Rejected: a span computed by a function, kept only for the two looks
that switched one millisecond late (a strict `>`); those two were made
inclusive instead, so the player is uniformly half-open.

*Keys on time, the grid kept*: during a use she woke only on the
1.4 s frame grid, so a key ending off it came late (the snack's
fridge, due to close at 1.5 s, closed at 2.8 s). The grid stays: the
TV's static and every bob move on it, and dropping it for still keys
froze the TV. Both are anchored at the start of the part playing, so a
prelude never shifts the use's own bob or channel. That changed the
trajectories, and was re-recorded on its own, for that reason alone.

*Choices hashed from her latest decision's whims*: arrival (starting
the use she walked to, coming out of a door) runs in the tick with
only the body's random stream, and a draw added there would shift
everything she does after it. So she keeps the whims of her latest
decision, and each choice at an act's start hashes them with its own
label: no body draw is added or removed. Rejected: threading the
choice through the job or the heading she's on: finding a heading
again rebuilds its job from what it's for, which would lose the
choice, and an act started again would roll its splice twice. A use
she resumes after an interruption goes through a new decision, so it
may be spliced again; "never twice" holds within one act. Before her
first decision her whims are her mind's seed, salted (an errand can
bring her out of a door first), and a stage cue forces a script
rather than rolling it, since the whims it would roll are the last
decision's.

## A splice never changes what it wraps (2026-10-03)

**Rule:** A prelude or coda (a splice) wrapped round a use changes
nothing about the use itself: its keys, length, credit, events and
what the shopping channel sold her are the same with or without it;
only when the use starts (after a prelude) and when the act ends
(after a coda) move. Only the use is credited. No splice wraps
unpacking or crumpling. The shopping channel's purchase is still made
the moment the channel comes on. See [design.md](design.md#houseguest).

**Why:** a vignette is decoration: adding one must not retune her
needs, her credit or her home. So the body is drawn exactly as it
would be without the splice, and timed (keys, bob, grid, grievance,
credit) from where the prelude ends. Credit is by the share of the use
itself done: interrupted in a prelude, none (she hadn't started); in a
coda, all of it. The property tested is the unit one, a use started
with a splice forced on and off from the same state: the random
stream after it, the use's length, purchase and events, what she does
and shows at every moment of the use, when she wakes, and the credit
at sampled times are all equal. Rejected: comparing whole visits with
and without splices, which can't hold: her needs rise with time, chat
arrives at fixed times, and visits diverge after the first wrapped
use. Unpacking and crumpling fire their events (the piece out of its
box, admiring what she made) at the act's end, so a coda would delay
them; no splice may name them.

*The purchase at the start*: the proposal moved it to the advert's
first key. Kept at the moment the channel comes on (the rule since
phase 3): a purchase at a key would be lost to a chat line arriving
before it, so what she buys would depend on when people talk, and
nothing a vignette adds needs it later.

## Many of her lines come from pools, and only beats have a budget (2026-10-03)

**Rule:** Many of her lines are drawn from pools (beat lines, door
lines, musings, riddles), each pool's rolls salted by its own id. A
pooled line isn't said again within ten minutes, whichever pool it's
from, and counts as said whether or not it showed, except one spoken
over in the same instant it's said, which isn't said at all and doesn't
cool. Her fixed lines (greetings, "Ow!", "Sata andagi." and the like)
are in no pool and have no cooldown. A visit has eight beat lines at
most; the other pools have no budget. No splice plays again within ten
minutes, and no channel surf. A door line comes one door in three.
See [design.md](design.md#houseguest).

**Why:** *A budget for beats only*: the budget is there so the beats
she owes for what she lost don't nag. Vignette pools are paced by
their own chances and cooldowns; one budget shared with them would let
a chatty visit's musings and door lines silence her beats, or the
reverse. *Salted by pool*: two picks in one decision (a door line, then
"Ah, right!") would otherwise share a roll. Beat lines keep id 0, so
their rolls didn't change.

*A cooldown per script*: the line cooldown can't pace a script whose
lines are fixed (surfing says nothing; the andagi says the same line
every time), so the store keeps a ten-minute cooldown per script, and
the splices and surfing consult it, which bounds surfing, the
chopsticks and the andagi alike. A splice or surf that doesn't roll its
chance hasn't played, so it doesn't cool. The other scripts don't need
it: each riddle's question is a pooled line, so it cools, and the
pool's chance paces them; the shopping channel comes on only on a
visit it's due; and bedtime is how every sleep looks, so cooling it
would only make some sleeps plain. Rejected: a script cooldown for
riddles as well: one musing in three on a third of her spacing-outs
already makes them rare, and the line cooldown keeps any one riddle
from coming round again.

*Spoken over in the same instant*: said is said, as with her
grievances, so a pool doesn't repeat a line just because its bubble
had no room. But a line drawn and spoken over in that same instant
(the door's line, then the decision she makes coming through) never
could show: nothing is drawn between. Cooling it would spend one of
the pool's few lines on nothing.

*One door in three*: the simulator heard "Where was I?" 467 times in
16 half-hour stage visits. Five lines on ten-minute cooldowns without a
gate come in bursts: five doors talk, then the rest are silent until
the cooldowns pass. Gated, a stage visit hears about eight, and the
door may now be silent. Rejected: a line at every door (one in one),
and no gate, with the bursts accepted (five doors talk, then silence
until the cooldowns pass).

## She answers a question with a sata andagi (2026-10-03)

**Rule:** While she names a sata andagi after a snack, a chat or IRC
line that asks her something (ends in `?`) gets "Sata andagi." said
toward the chat, and she plays on; any other line, or one during the
snack itself, stops her as any line does. A line asks only when the
count of its source rose and that source's newest line ends in `?`.
See [design.md](design.md#houseguest).

**Why:** the user's call (2026-10-03): it's the joke. The coda carries
on and isn't shortened, so the answer is part of it, not an
interruption. *Per source*: arrival is seen as the chat mark changing,
and a compaction changes it too, so one "newest line asks" level would
have answered a compaction while the newest line was a question; and
"the newest synced or IRC line" has no defined order between the two.
So each source carries its own level, and only a source whose count
rose is asked.

## Deferred from the vignettes: Adverb, and rarity (2026-10-03)

**Rule:** Phase 5a has no `Adverb` (a manner a vignette is played in).
For 5b: a want's tier or rarity must gate whether it's *offered*, not
scale its score. See the
[proposal](proposals/2026-10-02-houseguest-mind.md#vignettes) (its
migration row 5).

**Why:** *Adverb*: her moods already carry her traits as rates (lazy,
industrious, dreamy), and no 5a vignette needs a manner of its own.
*Rarity gates offering*: a choice multiplies an offer's factors,
truncates to the top four, then rolls. A factor below 1 doesn't make a
want rare; it removes it whenever four others outscore it, and does
nothing when fewer are on offer. So rarity and pity must decide whether
it's offered at all (at the offer filter, or when her mind binds it).
*(2026-10-04: 5b gates scripts, not wants: a rare splice, musing or
dream is passed over before anything of it rolls, and no want is rare;
see [rarity is drawn per day](#rarity-is-drawn-per-day-and-pity-is-only-for-the-unseen-2026-10-05).)*

## Her days run on a game clock (2026-10-04)

**Rule:** Her routine runs on a game clock of her own: six times real
time, running only while dessplay is open and only once she has met
you, starting at Monday 16:00 and never resynced with the real time of
day. Moving her out sets it back to Monday 16:00. Only the real *date*
reaches her (her calendar, the school holidays). See
[design.md](design.md#houseguest).

**Why:** the user's call (2026-10-03). The proposal's routine ran on the
local wall clock, which makes her a schedule nobody sees: a client
opened for an evening's episode would only ever meet her evening, and
one opened at work would find her at school every time. A clock that
runs only while dessplay is open makes every session see her day move,
and at 6× a game day is four real hours, about a long session. 16:00
because she's home, awake and in her afternoon (the slot whose levels
are exactly her pre-clock arrival levels), with her evening and first
bedtime coming within the session. Never resynced: a game clock that
jumped to the real hour would undo the point. Only the date crosses,
because a calendar is only fun on the real day (Halloween on the 31st),
while a time of day would bring the wall clock back. The clock stands
still until she has met you, so a client that never met her writes no
record, and her first meeting is at 16:00. Rejected: the wall clock
(above); a clock that runs while dessplay is closed (the wall clock
again, sped up).

## Her clock is stored as game minutes since Monday 16:00 (2026-10-04)

**Rule:** Her record keeps her clock as whole game minutes since Monday
16:00 of game day 0. That start is part of the format and never
changes. A record without the field, or with anything but a whole
number below 2^40, reads as Monday 16:00. See
[design.md](design.md#houseguest).

**Why:** *Game time, not real time*: storing real milliseconds would
tie her past to the 6× ratio, so retuning it would move where she is in
her week. Game minutes stay right whatever the ratio. *From Monday
16:00*: a new record is all zero, as it was before the clock, so every
pre-clock record reads as a new clock, which is right for a new home
and acceptable for an old one (the changelog said so). *Lenient*: like
the rest of the record, a field it can't read is the default rather
than a failed record. Rejected: game time since a Monday-00:00 epoch
(as good, but a new record would no longer be all zero); a separate
settings key for the clock. A separate key would survive a downgrade,
but needs a second write and a second read, and moving out would have to
clear both.

*The downgrade loss*, accepted: an older build ignores the new fields
and drops them when it saves. After running `stable` and coming back,
her clock is back at Monday 16:00, her pity and the rare things she has
shown start again, and her calendar may greet again that day. It also
skips the pieces it doesn't know (the window, the wall clock) and a
window on order, as it skips any unknown piece. Each is small, and none
breaks the record.

## Her clock has no clamp but the shell's ten-minute step (2026-10-04)

**Rule:** Her clock counts the UI thread's monotonic time. One step of
it is capped at ten real minutes only when the shell asks
(`Guest::cap_steps`), which it does, logging at debug when the cap
bites. The cap is off by default: tests and censuses jump hours on
purpose. A capped step also counts at most ten idle minutes toward her
pity.
See [design.md](design.md#houseguest).

**Why:** A suspended laptop shouldn't age her a day. std's `Instant` is
`CLOCK_MONOTONIC` on Linux and `CLOCK_UPTIME_RAW` on Apple
(`library/std/src/sys/time/unix.rs`, Rust 1.98), neither of which
counts a suspend. But std says "it is also not specified whether system
suspends count as elapsed time or not. The behavior varies across
platforms and Rust versions" (`library/std/src/time.rs`), and Windows
is unchecked. The UI thread steps at least once a second, so ten minutes
never bites in normal running; when it does, a whole suspend adds at
most an hour of her day. The cap lives in the guest's accrual but is
off unless the shell sets it, so a test helper that jumps hours still
counts all of them: a cap always on would lose game time there
silently. The idle minutes are cut with the step, since a suspend is no
more idle time than it is game time. Rejected: no cap (trusting an
unspecified behaviour); a cap always on in the guest.

## Her record: events at once, time in batches, and on exit (2026-10-04)

**Rule:** What happens to her (a purchase, a parcel, a piece set down,
something rare seen, the calendar delivered) saves her record at once.
Time alone saves it every 30 game minutes (5 real) and at each change
of routine slot. Every way out of the UI loop hands back her final
record, its clock brought up to that moment, and the pane sizes, for
the caller to save after the UI thread has joined. A SIGHUP, a panicked
UI thread, and a failure between the UI's start and the session's end
(the `?` exits in `run_interactive`) stay unsaved. See
[design.md](design.md#houseguest).

**Why:** *Batches*: her clock changes every second; saving each change
would be a settings write a second for nothing. Thirty game minutes is
what a crash loses at most, and the slot changes keep the record right
at her departures and bedtimes. *The exit save*: before it, every exit
of the UI loop (Quit, Shutdown, a closed channel, a failed draw) skipped
the loop's one save, so a change in the last iteration was lost, and so
was a handout parked on a full action queue. The loop is labelled and
every exit is a `break` to its one end, which hands back the record:
an exit that skips it can't be written. It doesn't go through the
periodic handout's "unsaved" flag, because a parked handout has already
cleared that flag; it compares with the record restored at startup instead,
so a client that never met her writes nothing. The pane sizes had the
same bug (saved only on the loop's periodic path), so they go back the
same way. The unsaved exits never join the UI thread (a SIGHUP exits at
once; the `?` exits don't restore the terminal either), and a panicked
UI thread hands nothing back. Rejected: saving on exit
only (a crash loses the parcel that just came); an inner function for
the loop body (clippy's `too_many_arguments`).

*One process per record*: `instance_lock` lets one process hold a
database at a time, so two clients never count time into one record and
then save over each other.

## Away at school ends the visit at her door (2026-10-04)

**Rule:** When she leaves for school the visit ends, once she's through
her door. Her home stands empty until 12:45 with her closed door where
she left; she comes home out of it as a new visit. Every routine exit
and return is by her door, never a screen edge. See
[design.md](design.md#houseguest).

**Why:** *A visit's end, not a long hidden act*: everything kept per
visit (the mood and hello, her beat lines, her shift, a parcel's
arrival, what's rare) stays meaningful. Keeping her "in" a 4½-hour
hidden act would need each of them re-keyed, and every check of what
she's doing would have to know she isn't there. The cost is honest and
small: a resident's school day counts one visit more (her return), so
shopping comes a little sooner. *The closed door*: the user's call
(2026-10-04). An empty home says nothing about where she is; her door
standing closed where she went out says "out, back later", and her
coming out of it says "home". So it stays until she comes back out of
it, follows a resize only when its spot no longer fits, and rains out
of a resident's focused pane with her pieces. *Her return is its own*:
it isn't called off by a key or a chat line (they'd lose her
homecoming for good), only by the gate closing (playing, an overlay,
visits off); then she arrives later through the idle gate.
*A short school day* (08:30 to 12:30, home at 12:45): the user's call.
The proposal's 08:30 to 15:30 would leave every other two-hour session
mostly empty or dark. Rejected: a hidden act (above); leaving by an
edge (no cue in the empty room). *Superseded in where, not whether
(2026-10-08):* her closed door no longer stands where she went out; it
stands at her door's space by the screen's edge, or the strict fallback
(see [Her door has its own space at the screen's edge](#her-door-has-its-own-space-at-the-screens-edge-2026-10-08),
*Where her door stands*).

## At school or asleep, she still comes, by her door (2026-10-04)

**Rule:** The accordion's errand still comes whatever the idle gate
says, but how changes with her day. At school she dashes in through her
door, pokes it, and goes straight back out by her door, not counted as
a visit. Asleep, she gets up groggy, pokes it, and goes back to bed.
One school day in three she also dashes home for her lunch, at a minute
fixed by her home and the day, only when her clock runs across that
minute. See [design.md](design.md#houseguest).

**Why:** the user's call (2026-10-03): the errand is a service (someone
said something while you were scrolled back), so it can't wait for 12:45
or the morning, but she shouldn't stay. The dash is the same visit:
not counted, so nothing per visit moves (her shopping, her mood); its
body stream is reseeded from its own salt, so the visit after it draws
as it would have. *Only when her clock crosses the minute*: a cold start
or a restart after it would otherwise dash at once, every time. Drawn
from the home's day seed, not a random stream, so it's at most one a
day by construction and no draw shifts anything else.

## Overlays end a dash and an errand (2026-10-04)

**Rule:** An overlay (a modal, the settings) ends a dash home and an
errand under way at once, as it ends any visit: her things rain out,
and if she hadn't poked the accordion yet, it shakes by itself. A
visitor's video or a held selection still lets her finish. See
[design.md](design.md#houseguest).

**Why:** the dash and the errand stay through a visitor being busy,
because they're brief and on her way. That exemption had been written as
"through anything that closes the gate", which let her carry on over a
modal: nothing of hers is drawn over an overlay on any other visit. The
dash and the errand were the same bug in two places, so both were fixed
together.

## The day is the unit (2026-10-04)

**Rule:** With her clock running, her mood, what's rare and her budget
of beat lines are the game day's, not the visit's: keyed on her master
seed and the game day, drawn at the day's first visit or as she wakes
into it, and carried over the day's later visits. Her body's random
stream is still the visit's. See [design.md](design.md#houseguest).

**Why:** *Same day, same Osaka*: keyed per visit, the 12:45 return
would bring a new mood an hour after "Mornin'... lazy day.", and two
visits an hour apart would greet you twice in different moods. *Each
morning a new day*: the user's call (2026-10-04). A resident who keeps
her through the night sees her wake into a new mood with a new budget,
as a new visit would bring. *The body per visit*: keying it on the day
would make two visits the same day walk the same steps.

*Carried in memory only*: what a day hands its next visit (the line
budget, the rare draw, the night's Dream and snack, the meal lines) is
kept per process, not saved. A restart the same game day starts them
afresh: the mood is the same (it's keyed, not carried), but her beat
lines are full again, and the day is drawn again (a second new rare
thing is possible). Saving them would put five per-day fields in the
record for a restart on the same game day, which is rare at 6×.

## Her holiday flag is held for the game day (2026-10-04)

**Rule:** A game day takes its holiday flag from the real date the
first time it's read, and keeps it until the next game day. The flag
isn't saved: a restart reads it again. See
[design.md](design.md#houseguest).

**Why:** the real date changes at real 09:00, in the middle of some
game day. Read live, a date flip could cancel school while she's at
school, or move her bedtime after she went to bed. Each game day keeps
its own flag, so slots are a pure function of game time within a day.
Her bedtime asks whether tomorrow is a school day before tomorrow's flag
is read, so it uses today's date; the next day's own flag may differ. A
school night whose morning turns out to be a holiday has her up at
09:00, not 07:00 (105 real minutes asleep), and a holiday's last
evening before school has her up at 07:00 (75): a night is 75 to 105
real minutes, not at most 95 as the design assumed. *Not saved*: a restart is a cold
start, which works out where she is from game time and the date as it
is then (she isn't mid-act after a restart), so re-reading the flag
changes no slot under her. *The evening before a day off* (her book's
boost) is judged the same way, by the evening's own flag: Friday's
evening and a holiday's last evening are evenings off, Sunday's is a
school night, and so is the evening before a holiday begins.

## Her calendar's day starts at 09:00 (2026-10-04)

**Rule:** The real date she gets is `timeutil::biblical_date`'s: the
day starts at 09:00, as the chat's day separators do. Something a date
owes her is owed once a day, and delivered only when it shows. See
[design.md](design.md#houseguest).

**Why:** watch parties run past midnight. Halloween at 00:30 is still
Halloween to the people watching, and Christmas morning before 09:00
is Christmas Eve's night. One day boundary for the whole client
([the chat's](#day-boundary-at-0900)), so the chat's "today" and hers
agree. *Delivered when shown*: marked when set,
a greeting with no room for its bubble, or one cut off by a key, would
use up the day unseen. The date is saved in the same paint that shows
it, so no visit's end can lose it.

## Chat at night makes her stir, not wake (2026-10-04)

**Rule:** Asleep for the night, a chat line makes her stir ("mm...", a
blink, turning over) and sleep on, whatever it asks. By day, a nap or a
doze still looks at the chat (until 2026-10-06: since then a doze by day
stirs too, saying "Mm?"; see
[In a still act she looks up where she is](#in-a-still-act-she-looks-up-where-she-is-2026-10-06)).
See [design.md](design.md#houseguest).

**Why:** a watch party chats most at night, which is exactly when she's
asleep. If each line woke her she'd never sleep through a session, and
her night (bed, sleep-talk, the Dream) would be cut every few seconds.
Stirring shows she heard it, which is what the look is for. Rejected:
waking her (above); not reacting at all (the chat would seem not to
reach her).

## Her night is one act, and her lamp stays off (2026-10-04)

**Rule:** Her night's sleep is one act until her wake time, whatever she
sleeps on: her bed, her sofa, a heap she made, the floor. The lamp goes
off as she settles and stays off until she wakes, whatever gets her up
in between. Bedtime alone doesn't turn it off. See
[design.md](design.md#houseguest).

**Why:** *One act*: her night's sleep-talk, the stir and the Dream live
on its script, so they come wherever she sleeps, and every way of
getting there (a bed, the floor because there's nothing else) has one
end, her wake. *The lamp latched*: tied to the act, the lamp came back
on for the groggy errand and the midnight snack, so in the middle of
the night she'd pad about with the lights up. Latched by the night's
lamp key and cleared at the wake, it's dark for everything between.
Bedtime alone doesn't turn it off, so the moment she switches it off
as she settles still plays.

## Rarity is drawn per day, and pity is only for the unseen (2026-10-05)

**Rule:** Only rare and legendary scripts are gated. Common and
uncommon ones play by their own chances. A game day (a visit, without
her clock) draws once which rare things are open: those she has shown
you, each tier at its base chance, and at most one she hasn't, at the
base chance plus her pity, certain at the tier's bound. Pity only
chooses something unseen. See [design.md](design.md#houseguest).

**Why:** *Gated, not scaled*: a factor below 1 doesn't make a want rare
([2026-10-03](#deferred-from-the-vignettes-adverb-and-rarity-2026-10-03)).
*Common and uncommon left alone*: their chance rolls already are their
rarity, and a second system on top would make them rarer than tuned.
*Per day*: the day is the unit (above), and a draw per visit would make
rare things as common as visits, which a resident and a visitor have in
very different numbers. *At most one new a day*: an `Option`, so two
unrepresentable; something new is an event, and two in a day halves
each. *Pity only for the unseen*: the first design rolled both the seen
and the new at base plus pity. But pity starts again only when
something new is shown, so once every rare had been seen the counter
ran on forever, and past six idle hours every seen rare was open every
day: rare stopped being rare (the cliff, found in review). Seen ones
now roll the base chance alone, and with nothing unseen left there's no
pity at all. Pity counts real idle minutes, not game time, so a fast
clock doesn't make it more generous. *Drawn once a day*: a visit begun
after midnight draws the day over the night and the morning, and her
waking doesn't widen it to the whole day: a second draw would roll the
day twice (a second chance at something new, and what's open changing
under a visit that already had it). So an evening's rare thing can't be
new on such a day.

## A parcel comes only when she's up and can stand to unpack it (2026-10-05)

**Rule:** Every delivery (her TV, a purchase, the wall clock) waits
until she's up and in sight: not asleep or up in the night, not on a
dash home, not between her doors, and after "I'm home!". It comes in
only where she can stand to unpack it, judged by her seat test (a floor
under her, lines allowed). The wall clock also waits for her hello, the
day's calendar, quiet, and no other parcel's flap still open (every
parcel comes through a wall's flap, never her door). See
[design.md](design.md#houseguest).

**Why:** a parcel is announced ("A parcel!"); delivered while she's
asleep or out, the line is said to no one and the box sits there with
no story. Asleep, she'd have to wake to say it. *Where she can stand*:
a hung piece's box is small (the clock's is 3 wide), and judged by
"room for her box" it never fit at a wall, so the clock never came;
judged by nothing, a box could arrive where she couldn't get to unpack
it. Her seat test is the one every use is judged by. In the same review,
a boxed cat bed counted its cat and was never unpacked, which stopped
her shopping for good (a bug since 2026-10-01); she now unpacks it like
any box. *The clock after the calendar*: the date's greeting is owed
first, all of it, and a parcel's line would talk over it. A day she lets
be, or a New Year's sunrise her TV can't be reached for, doesn't hold
the clock back.

## Her window and her wall clock (2026-10-04)

**Rule:** Two hung pieces show her time of day: the wall clock (to the
quarter-hour) and the window (five skies). Both are always drawn facing
one way. They change on her clock's quarter-hours, and only a home
showing one wakes for them. The wall clock is a gift, once, after her
TV; the window is sold after the cat bed. See
[design.md](design.md#houseguest).

**Why:** the user's call (2026-10-03): game time needs to be legible,
or her bedtime at real 20:00 looks like a bug. *Never mirrored*: a
mirrored dial reads 3:00 as 9:00, and drawing one way also halves their
images. *Quarter-hours*: 48 faces in all, changing about every 2½ real
minutes, and her frame cache holds them. Homes without either gain no
wakeups, so nothing else redraws. *The clock as a gift*: it explains her
first bedtime and first school morning, so it should come soon to every
home, old ones included, not wait its turn in the shop. After the TV,
because before that her home is a first meeting or two. *The window
sold*: after the cat bed, before the decor, so homes that have a cat
bed are the only ones whose shopping changes.

## A window comes in where she can look out of it (2026-10-05)

**Rule:** A window is delivered first where she'd have a spot to look
out of it (clear floor under it, or just beside it). Only with no such
wall does it come in where she can't, over a sofa. See
[design.md](design.md#houseguest).

**Why:** a window she can't look out of is a picture. The test
measured 26 of 40 delivered windows with a spot to look out from before
this, 40 of 40 after. Over a sofa is kept as a fallback, rather than
refused: making it strict would also stop her moving a piece under a
window when she puts her home right, since both use the same room
test.

## A chat line's watch is 5 s (2026-10-06)

**Rule:** After a chat line she looks (4 s: `!`, then `?`) and watches
the chat until it has been quiet for 5 s, then carries on; each line
renews it, so a lively chat still holds her. See
[design.md](design.md#houseguest).

**Why:** the user, for phase 5c (stillness): "the act of looking is
there solely to draw attention, which only happens during change." The
look (the turn, `!`, `?`) is the change; the 11 s she then stood
watching held her out of whatever the line had cut, and in a room with
a line every minute or so that was a large share of her time. 5 s keeps
a second of plain watching past the look, so a line's look still ends
on the chat, and a lively chat (lines closer than 5 s) still holds her.
Rejected: no watch past the look (between lines of a lively chat she
would set off, to be stopped again by the next).

The watch is counted from the line even when her look is put off: on
a pole or in the air, in a door or out, passing over text on her way
to somewhere calm, or answering with the andagi. With 15 s that
deferral usually fit inside the watch; with 5 s it often doesn't, and
then she lets the line go without turning to it. That is kept on
purpose (the same quote: a line some seconds old is no longer change,
and turning to it then draws the eye for nothing). Rejected for now: a
watch owed until she can first see the chat (a flag set on those paths,
starting a fresh watch at her first restful decision); phase 5c's step 6
(she looks up in place) reworks how a line meets what she's doing, and
can revisit it there.

Until that step lands, the shorter watch hands the freed time to fresh
decisions (often a walk, 5 s after the line instead of 15), so in the
9-minute census chat adds set-offs over a quiet room in the home and
resident (home, lazy: 2.06 a minute against 1.51) where before it took
some away: expected, not a regression; looking up in place is what is
meant to make chat cost her no movement.

## In a still act she looks up where she is (2026-10-06)

**Rule:** A chat line during a still act on calm floor (sitting, lying
or gazing on the floor, spacing out, or a piece she rests at: lounging,
napping, a day's sleep, homework, watching TV, reading, looking out of
the window) doesn't stop it: she looks up where she is, in its pose,
turned to the chat unless she's lying down or at her desk or a sill
(`!`, `?`, then watching plain-faced until her watch ends, each a
frame at least: below), and the act
runs on with its own time and what it eases her by; at a piece she
turns back to it after. Dozing, she only stirs ("Mm?", as at night),
with no watch after. Walking, pulling, chores, mischief, making, moving
a piece, exercise, standing about, Setsubun's beans, a dash home and
anything over text are cut as before. See
[design.md](design.md#houseguest).

**Why:** the user, for phase 5c (stillness): "the act of looking is
there solely to draw attention, which only happens during change." The
old look stood her up: from a sofa, a nap or her homework she got up to
a standing side view for the look and the watch, then chose afresh
(often a walk), so each chat line cost her the act and two of the
biggest changes her sprite makes (lying or sitting to standing, and
back down to something else). With the phase's longer still acts, those
acts catch more lines. Looking up in place keeps the look (the turn,
`!`, `?`), which is the part that tells the viewer she noticed, and
drops the getting up. The andagi answer and the night's stir already
reacted in place this way.

The classes are fixed in one place: the act's chat class (still or not,
by the act and, for a use, by what the piece is used for) and, for
dozing, the pose she's drawn in at the line (so nodding off over her
homework stirs, and new doze poses class themselves; nodding off under
a look ends it, since a doze never wears one). A script can override a
still act's class: Setsubun's beans are thrown on the spot (a
SpaceOut), but they're throwing, not stillness, and the dash home for
something forgotten is a dash, so both are cut as before (a script's
own chat class, `Chat::Stop`, beside the act's). Lying down she keeps
her facing: on her back her head is already at the end away from her
facing, and turning over to face the chat would be the very change this
removes (the user approved the art that way). Lying on her front is
held to the same, as the same end-to-end flip, *pending the user's
word* (the approved sheet turned only Sit, CrossLegged, UnderSill and
Lounge, and exempted lying on her back). Poses aimed at a piece (her
homework and chopsticks at the desk, the paper desk, leaning on a sill)
keep their facing too: mirrored, her arms would reach into the air with
the desk behind her; she looks with her face and bubble alone. At a
piece she turns back to the seat's facing once her watch is over, or
she'd sit with her back to her desk or TV for the rest of it.

What her act says under the look is hidden; every line her act's
script said under it is said once the look is over, in turn (a
riddle's question, then its answer: the answer alone would be a
punchline without its setup), or as the act ends if it runs out first
(a short spacing out ends inside the look's 4 s). What she's saying
herself shows over the look, as it did over the standing look. A line
as she says what's wrong with her home doesn't cut it short: the look
waits until she has said it (two frames), so it is felt and the `!`
still shows; before, a chat line cut it short and it wasn't felt (this
ordering is the implementer's call, open to the user). A stir sets no
watch: she never turned to the chat, so waking just after she gets on
with her day rather than standing to watch it. A chat line no longer
cuts a still use, so a day's sleep on a bed she made runs its course
while another piece she made waits (the made-piece property pauses the
other pieces' waits while she uses one she made).

Rejected: resuming the act after a standing look (the design's first
answer, D3): it still stood her up and lay her down again for every
line, and needed guards for a moved seat, the routine, a script
replayed from its start and credit scaled by what was left. The census
counts an in-place look as a look that cut nothing (no restart after
it); a stir isn't counted as a look, as at night.

**Each of her look up's marks shows a frame at least (phase 5c's tail,
T4, 2026-10-08):** her `!` showed 1.2 s and, once her `?` was over at
4 s, her plain-faced watch to the line's 5 s watch 1 s, each under the
1.4 s frame, so each came and went inside one, the flicker T2 ended for
whatever she says (a stir's "Mm?"). Now her look up is startled a frame
(`LOOK_UP_SURPRISED_MS`), puzzled to 4 s as before (2.6 s), and
watches plain-faced until her watch is over and a frame past her `?` at
least (`LOOK_UP_MS`, 5.4 s from the line): 400 ms more than the 5 s
watch of a single line, the "second of plain watching past the look"
(above) now a frame. Rejected: ending her `?` a frame before the 5 s
watch (it keeps the 5 s, but where her look waits for her grievance
the watch can be as short as her `?`, and then one of the two would
still be under a frame). Only the look up changed: her standing look
(`!` 1.2 s, `?` to 4 s, or 0.8 s after something got her up restless,
her seat gone or shaken, then watching on her feet) is a short act that
moves, as her walk is, and the stillness rule is for still acts over
30 s. A line in a lively chat restarts the look (its `!` cuts the mark
it shows short): that is the user's line, which the look exists to
answer. Other marks and keys under a frame, and why they're left:
her slow blink (an exemption, steady and periodic); the switch-on's
static (three 400 ms frames in a watch's first 10 s, and channel
flicking, held by the stillness tests); the vignettes' quick keys
(chopsticks before homework, the andagi and the last melon bread after
a snack, Setsubun's beans, a clock glance), each in an act under 30 s
or in its first 10 s; peering over an edge, a glance at something
lost, mischief's recoil and oops, all on her feet. Not left by choice,
but open (T4's review; plan.md): her act's own keys under a look. A
key of her still act may start within a frame of a look's change, and
nothing holds it off: her homework nodding off ends the look (a doze
wears none), so a line a moment before it shows its `!` under a frame,
and a key's pose changing under the `?` or the watch comes less than a
frame after it. design.md's rule (whatever of hers changes next waits a
frame) counts it; a key is her act's scheduled change, so keeping the
two apart means putting off one of them (nodding off till the look is
over, or the look's next mark to the key), the user's call. The drawn
stillness test judges it (her pose changing under a look is no
exemption) and no run of it meets one. The look up waiting
for a musing moves it 400 ms later (`a_daydream_session_says_its_musings_in_turn`).
Test: `her_look_up_shows_each_of_its_marks_for_a_frame` (every still
act, a line, her face and bubble every 10 ms to a frame past the look),
red at the `!` (sitting: 1200 ms) and, the `!` fixed alone, at the plain
watch (lying on her front: 1000 ms). Golden traces: 69 moved, every one
first at a look up's `?` starting 200 ms later (since + 1.4 s) and its
end 400 ms later, the sequence of her acts and places unchanged in all.

## Her stillness levers: settling in, lingering, daydreams, nearer spots (2026-10-06)

**Rule:** A still act she chose that runs its course may settle
further where she is (spacing out or gazing to sitting; sitting to
lying back or dozing where she sits; lounging to a nap on the same
seat), skipping the roll; her mood stretches the still acts she chooses
and moves where her homework nods off; a musing daydream holds several
musings; a line to pull, letters to swap and text to make a piece of
are on her own floor first, and nearer spots are likelier. Each lever's
values come with the stillness band's tuning (phase 5c step 8); they
landed neutral, and shipped in step 8c ("Her stillness ships", below).
See [design.md](design.md#houseguest).

**Why:** phase 5c's band asks for less moving in sight, and lazy moods
noticeably stiller than industrious ones, without a mood factor on
walking (the user, Q2: the difference between moods comes from
still-act lengths and settling in). Need rates alone gave about four
points between lazy and industrious (the map's C3), since every use
begins with a walk; a mood that lingers and settles further spends
longer between walks instead.

**Settling skips the roll** because it's a continuation of what she
chose, like a leftover or a heading, not a new choice: she sat down
because spacing out ran on, she lay back because sitting did. Rolling
afresh at each step would make every settled act compete with a walk
again (the roll's top four almost always holds one), which is exactly
the restlessness the band measures; and a settled act isn't one of her
recent choices, so it doesn't make the act she'd choose next less
likely. It binds only what her methods would bind there (a nap only
from the same seat, as napping there is on offer), and eases her as its
own want. It sits after everything that already pre-empts a choice
(her routine, a beat she owes, a heading, a piece she's moving or made),
so bedtime, school, a glance toward what she lost and arranging win.
Watching the chat as it ran out, she settles first and watches on in
the new pose: standing up to watch and then choosing would undo the
point of looking up in place. But not when one of those pre-empting
things is waiting (review, 2026-10-06): settling first then held it
back a whole settled act (a nap is a minute or more) instead of the
five seconds of a watch, so a glance toward something she lost came
long after the loss, and which came first hung on whether a chat line
happened to be up. There she stands the watch out, as before settling
was, and the waiting thing is next; the check is the same test each of
them makes where it's done. Only
what she chose settles (not a glance at her clock, Setsubun, the moment
after a swap or a trial sit), only an act that ran its course, and only
on calm floor. Dozing where she sits (the approved sitting doze) is a
second branch from sitting, and an activity she never chooses: it's
not among her wants, and eases her as lying back does.

**Lingering applies where the length is drawn,** for each act she
chose, never inside the length tables: a glance, Setsubun, the moment
after a swap, a trial sit and her night keep theirs. Lying on her front
kicking her feet doesn't linger (twice a second, a longer kick draws
the eye longer), nor does exercise. Homework moves its nod-off instead
of lingering: its slot already makes it long, and where she nods off
is the mood's tell.

**A daydream is one act**, its musings said in turn on its own clock:
each musing as a fresh muse would have ended the act (crediting its
first part and clearing what it eases) and drawn every musing from the
same whims (a session all riddles or none). A riddle, her rare musing
and a glance at her clock are only its first; a musing due while she
speaks or looks up waits. How many, and the gaps, come from her
decision's whims, not her body's stream, so they're the same however
she's drawn. Musings that don't fit in the daydream's drawn length go
unsaid, so the band's guard on the longest still act reads the length
tables alone (the band guard counts a chain she settles along as one
stretch, since a settled chain is one still stretch to the eye). Her
rare musing and a glance at her clock roll before the musing count
(review, 2026-10-06): they aren't musings of hers, and 5b pinned their
frequency (the escalator's rarity and pity, the clock glance once a
visit); rolled after it, a mood whose daydreams are often silent would
see them a third to a half less, for no reason of the mood's. Eight
musings cooling ten minutes each means back-to-back daydreams run
dry: a long session says less the more of them she's had, which suits
a daydream (it goes quiet) and keeps lines rare.

**Nearer spots, strictly on her floor first for text:** a weight of
four for her own floor would only have moved her from a fifth to half
of her pulls (the stage's off-floor hops were most of its moving); on
her floor first makes "pulls stay on her floor when there's text there"
true. A seat is weighted, not filtered, so every place she'd use stays
on offer. Wandering keeps no distance weight: shorter wanders lower the
share without lowering how often she sets off, which the band's cap
counts.

## A slow blink on a pose she holds (2026-10-06)

**Rule:** On a pose she holds with her eyes open she blinks for 150 ms
every 6–12 s of the act, the gaps drawn from her deciding whims; not
while she looks up at the chat. Gazing says "ooh" for its first 3 s
only. See [design.md](design.md#houseguest).

**Why:** only standing blinked, so a minutes-long sit, lounge or read
read as frozen rather than still (phase 5c, Q3; the user chose a slow
blink). Slow and brief, so it reads as alive without drawing the eye
the way setting off does: the whole of phase 5c is about attention, and
a blink every few seconds, or a lasting bubble, would undo it. A gaze's
"ooh" is its moment of noticing; held for a whole long gaze the bubble
would be the busiest thing on screen (synthesis M8).

**A pure schedule from her whims**, not her body's stream and not the
wall clock: drawing it from the body's stream would shift every later
length she draws, so a blink would change what she does; from the wall
clock it wouldn't be reproducible. As a schedule it can't change her
behaviour, only how she looks, so the tick reports it without making
it an event (it wakes the shell; nothing of hers fires). Under her look
up at a chat line her face is the look's: that's all attention, so a
line come mid-blink cuts it, and a blink begun under the look shows
none of its remainder once the look is over (a stray tail of a blink
after the look would read as a flinch).

**Not on spacing out.** Its pose is standing, but the stand's own
quicker blink belongs to standing idle; spacing out is a vacant stare,
and an unblinking stare is the joke. Q3's list of held poses didn't
name it either. If phase 5c's tuning makes it one of her long still
acts (M2, D4), that's the moment to look again.

**Measured consequence:** two kinds, told apart. The blink's wakes
change only how she looks, but the golden and census harnesses deliver
a chat line or a cue at the first step that crosses it, so the extra
steps let some land sooner: harness quantisation, not her (in the
client a line arrives when it arrives). The gaze's 3 s frame is
different: it's a new event of hers (her "ooh" going), and at her own
events she acts on a world change she has seen (a pane gaining focus,
say) sooner, in the client too; resident seed 3 in ASCII goes to her
door 3 s sooner that way.

## An easing lands on her needs as they are (2026-10-06)

**Rule:** What she did eases her needs as they are when it's credited:
each is brought up to that moment first. See
[design.md](design.md#houseguest).

**Why:** needs rose only as she decided, and a credit (an act settled
as she leaves it, a shift as she comes home, a pull as she finishes)
landed on them as they were at her last decision. A need that rose to
full over a long act then came back full, its easing lost; one eased
below empty lost the excess, which rising afterwards didn't give back.
Phase 5c step 2 found it and held the fix for step 8, because her rates
had been tuned with the easing lost and `her_needs_shape_long_visits`
("she dozes more in a visit's second half") failed with it alone.
Step 8's tuning stopped (M3's cap waits on the user), so the fix landed
on its own in step 8's review, rather than waiting on a retune: a known
wrong order shouldn't wait on numbers. The long-visit claim was
measured, not retuned: over its old four visits she now dozes less late
(0.7×), over twelve 1.3× as much late, over forty 2× (34 of 40 more
late). So the rule's shape holds and four visits were too few to carry
it; the test runs twelve. Its census effect is under 2 points a cell
(phase5c/baseline.md).

## Her stillness band: per-mood caps (2026-10-06)

**Rule:** The stillness band is an aim, read on a fed afternoon per mood
and the same in every room, quiet or with chat: her moving in sight as a
share of her time in sight between a floor and a ceiling (lazy 5–17%,
ordinary and dreamy 8–26%, industrious 12–31%), her set-offs a minute in
sight under a cap (1.5, 2.25, 3), and industrious at least 1.6× lazy in
each room. Watching doesn't linger until her TV holds a picture. See
[design.md](design.md#houseguest).

**Why per-mood caps (the user's call, 2026-10-06):** a set-off (a walk, a
hop, a door) draws the eye the same in any room, so how often she sets
off is judged by her mood alone. The share keeps its ceiling (the user's
"about 15 lazy, 20–25 ordinary and dreamy, up to 30 industrious", a
point or two over), and its floor only says she isn't dead.

**Why the per-room cap failed** (synthesis M3, phase 5c step 4): each
room's cap was its baseline rate scaled by the target share over the
baseline share, so a lever that only shortened walks would fail it. But
that froze each room's seconds moved per set-off at the baseline's, so
the cap capped the share; and the levers shorten her trips without any
walk getting faster, by keeping her near her things (once she no longer
wanders off and travels, what's left is the short walk to a seat or a
line). Step 8's tuning found the rooms needing opposite moves: the stage
had to set off about three times as often as the home to reach its
floor, from one shared table of lengths.

**Not met yet (step 8b, stopped):** under a per-mood cap the most a cell
can move is its cap × the seconds a set-off moves her. On the stage
that's 2–3.5 s (a hop to a line on her own floor, a few steps to the
piece she made), which puts its bound a point or two over its floor in
every mood, while the home (8–11 s a set-off) has room to spare and its
industrious cell sat over its ceiling. Every lever that moved the stage
moved the home with it; the one stage-only lever tried (Tidy 90 s)
moved the stage only within noise, since she is time-limited there.
Within industrious the home moves 2.35–2.97× what the stage does across
the runs, against the band's own 31 ÷ 12 = 2.58: holding the home under
31 leaves the stage at 10–13, at or within about a point of its floor
of 12, and the tuning may not aim a cell at its floor. A room-selective
lever, or a different rule for the stage, is the user's choice
(phase5c/baseline.md, "Step 8b"). The user chose lower floors on the bare
stage and to ship ("Her stillness ships", below).

**The band in the gate (the user, 2026-10-06):** the band tests run in
the gate at two seeds a cell, where 3σ about each aim is 13 to 34 points
wide: they catch a gross excess of moving and the set-off cap, and
almost nothing else. The real check is full strength
(`CENSUS_BAND_SEEDS`, release, `--profile band`, N up to 525: minutes a
test), run at step boundaries and in a phase's census pass; the mood
spreads run only there. The user: "Seems fine. I'll know to check them
if something feels off." What the user signed off as short is named in
`tests/band.rs`'s `SHORT`, each ignored with its numbers, so the gate
stays green without hiding a new shortfall (testing-strategy.md, "The
stillness band"). Rejected: full strength in the gate (a quarter of an
hour a test) and a band of 3σ at two seeds read as passing evidence.

**Watching out of lingering:** her TV shows animated static until phase
5c's held picture (D7); lingered, a lazy day would watch the busiest
thing on screen half as long again. It joins the list when the picture
lands.

## Homework on the floor and the paper desk (2026-10-06)

**Rule:** With no desk standing she does her homework lying on the floor
(on her front writing, or one time in three on her back with the set
text), nodding off where her mood has her; the first one a visit ends
with "My back..." and her back aching for the visit. With no bookshelf
standing she reads on her back, and may settle into a doze under the
book. She can make a paper desk (a low 4×2 cube of torn text) and kneel
beside it for her homework. Making a piece is three times as likely
while she owns no real one of its kind, a paper desk three times again
while her back aches (still with no real desk). Watching the TV from
beside it rather than from a sofa she sits cross-legged. See
[design.md](design.md#houseguest).

**Why the floor ones are activities with their own methods** (phase 5c
D5 as amended by M10): `IDLE` binds unconditionally, so floor homework
and reading on her back each get a guarded method of their own. The
guard reads the pieces shown (`Chances::real`, and what she made), not
their seats: a desk whose seat text blocks still stands in the room,
and homework on the floor beside it would read as a bug. Reading on her
back is an activity, not a method of reading at a bookshelf, because a
use is bound to a piece and a seat she walks to; on the floor there's
neither, and the activity path brings its credit, its look-up-in-place
and the census with it (phase 5c 10b's borrowed line, which walks to
text, is the method of `Use::Read`).

**Why base 6, not the desk's and bookshelf's 8:** the floor is the
lesser place (lying on her front, the nearest pose, is 6), and at 8 the
two crowded out the rest of a bare room's floor: with needs low and
sleepiness high, each scored 0.8 against a doze's 0.69, and lying back
fell out of her best four altogether (`sleepiness_draws_her_to_lie_down`
read 0 doze both ways). Neither keeps the chat's factor: what she does
on the spot never takes her into the chat.

**Why her homework on the floor nods off in its own script, and reading
on her back settles:** homework at a desk already moves its nod-off by
mood instead of lingering (M8), and the floor follows the desk, so its
doze shows with the levers she ships with. Reading at a bookshelf
doesn't doze, so on her back the doze is a settling-in (as sitting's
doze is), shown when settling ships. An activity's own script (her
night on the floor had the only one, held still) now wakes her on a
key's frames while it bobs, so the writing animates.

**Why "My back..." once a visit, and only on an end:** the line is the
moment the ache is set, so it's said once; her routine cutting the
homework off (bed, school) sends her on at once, and the next one aches
instead. The ache makes the desk likelier still (the brief: "its
discomfort drives a makeshift desk"), but only while no real desk
stands: with one, her back draws her to it, not to making another (a
real desk can arrive mid-visit, after the ache). It isn't what first
makes the desk: see "Measured, not resolved" below. The ache lasts the
visit (D5's "for the visit"), so across a night's sleep inside one
visit it still aches; whether a night should cure it is the user's
call.

**Why the desk is `Furniture::Desk` in `MAKES`, not a new kind:** made
pieces live only in the visit, so reusing the kind needs no ledger
change. Each kind she makes now names its own shape, size, seat and
line (`scrap::Made`, `Furniture::lost_made`), wildcard-free: before, a
third kind would have silently taken the sofa's. She kneels beside it,
not in it (her box centred a column past its end, facing it, as at a
real desk), so homework's stool poses resolve to kneeling there in one
place (`at_seat`). The chopsticks are drawn on her stool, so a splice
row says whether it may wrap a made piece's use, and one predicate
(`Splice::wraps`) answers for a rolled splice and for a stage cue (which
then waits for a real desk).

**Why a stand under a live watch faces the chat, wherever it's set:**
standing while a watch is live she's drawn side-on watching it, but
only the watch's own stand turned her; a reflex that stood her first
("I'm home!", landing from a climb a line came during; "My back..."
too, were a watch live as her homework on the floor ends)
kept whatever way she happened to face, half the time away from the
chat, until the watch's own stand a beat later. One rule where an act
is set (`Osaka::set`) covers every such stand, now and to come.

**Why making ×3 is a want factor while she owns none** (critique C4):
with no real piece of a kind on offer, making it is the only place for
that use, so a weighted pick among places would change nothing; a factor
on the want, when its bind is making, does. It's above 1, so it never
pushes that want out of her best few. "Owns" is read off the frame: a
real piece shown, boxed or not (one closeted for want of room offers her
nothing either).

**Measured, not resolved** (for the user): she can only make the desk
where there's text to tear (no text, no offer to make it), and wherever
she can, the desk comes first. Homework at a desk (base 8, made with
making's ×3) against homework on the floor (base 6) is about 4–5× at any
hour (both carry homework time's ×3, which cancels), so she usually
makes the paper desk at her first homework of a visit, before any
homework on the floor; her back's ×3 then mostly lands on a desk she has
already made. The ache doesn't order the two. To make "a sore back,
then the desk" the rule, either gate making the desk on the ache, or
leave making's ×3 off the desk until the ache; both are design changes,
left to the user.

## A borrowed line to read (2026-10-06)

**Rule:** With no bookshelf standing and a line in reach that lends a
strip, wanting a read she borrows a strip of it one decision in three
(reading on her back the rest): she walks to it, reels the strip off
into her hands, reads it sat beside the tear, and slides it back. Text
is out of its line only while she holds it. A key press while she
tears chat text (for furniture, or to read) catches her out, as at her
other mischief there. See [design.md](design.md#houseguest).

**Why a method of `Use::Read`, split with reading on her back by one
whim** (phase 5c D5 as amended by M10): HG #72 is reading, so the borrow
is reading's own method (`use/borrow`, after the bookshelf's), with
reading's row, factors and fun. Reading on her back (step 10a) is an
activity, a different want, so "tried in an order" can't be a method
order: both would be offered and a bare room's reading would double.
One whim (`borrows_now`) gates both: when a line lends a strip, one
decision in three offers the borrow and not the book, the rest the book
and not the borrow; with no line to borrow, always the book. The borrow
is the rarer because it's a walk and the book is read on the spot
(stillness: a walk is a set-off, and her movement draws the eye). Her
base for it is reading's 8 against the book's 6, so where it's offered
it draws a little more than the book would have.

**Why beside the tear, her own floor first** (character m3): read where
she tore it, sliding it back costs no second walk, so a borrow is one
set-off. A line on her own floor is taken strictly first, whatever
`Stillness::near` says: it's the method's rule (one short walk), not a
lever to tune.

**Why her holding the strip is what keeps the text torn, not a flag of
the act's phases** (the class: never leave it torn): the act names what
she holds (`Osaka::holding`: text she's tearing for a piece, or a
borrowed strip, the same value all the while), and the guest puts back
whatever of it is out the moment she doesn't hold it, however that
came about: a chat line, her grip lost (a refused reel or slide step,
or the strip she reads changed or scrolled under her), someone at the
keys, text coming up where she sits, her pane focused, the stage
placing her. It looks twice a paint, as it reads the frame and after
the paint's own checks (recheck, eviction), so no frame shows text out
she has let go of (without the second look the property fails: text
scrolled up where she sits moves her on mid-paint, and the strip stays
out a frame). A goodbye, her
room going, a resize or switching off end the visit, and its text
layer with it. A loss is owed only for text actually dropped: a strip
slid back leaves nothing to put back. The stage's cue places her after
the frame's text is drawn, so a cue that ends her hold looks a third
time and draws the frame's text again from the frame as it came (a
review found the strip left out a frame there; the property has no
exception for it now).

**Why her grip is checked from taking hold to letting go, for all text
she holds** (review of step 10b): the first cut checked only the strip
she sat reading. Reeling it in, a line that scrolled under her had its
glyphs dropped by the layer's validation, and the next reel step tore
whatever now stood in those cells: she'd read a strip of another line.
Tearing for a piece and pulling a line had the same gap (a key press's
put-back in the chat, then the same frame's pull step heaving the line
off again from home). The class is "she goes on with text she has lost",
so it's closed twice over: `Osaka::grip` names, for every act at text,
what must still hold (the line as she found it while bracing, every
glyph out of its line once any has moved) and the paint checks it after
the frame's ops, every frame, against the real frame and her layer; and
a reel step past the first, or a heave past the first cell, refuses a
glyph that isn't already hers rather than taking it afresh (the same
paint's validation can drop one the moment before that step runs, which
no after-the-fact check can see). Five resident goldens moved by a
frame at a key press mid-pull: the line no longer flickers back out.

**Why a key press catches her out by where the text is** (review of
step 10b): what goes back is the text in the chat (`drop_in` keys on its
cells), so whether she was at it there is read off the same cells
(`JobRef::text_cells`), not off where her feet are. By her feet, text
outside the chat with her standing on its border caught her out for
nothing, and text in the chat with her feet outside it went back under
her while she reeled on and tore it straight off again.

**Why the borrow's start and a forced put-back log at info:** both
change what's in the user's panes (text leaves a line; text comes
back). Its other beats (reading, sliding back) log at debug, as a tear's
do.

**Open: does she turn to the chat when she looks up from a strip?**
`ReadStrip` keeps the art commit's turning rule, so a chat line turns
her (strip and all) toward it, and she turns back to the tear when the
look ends. The desk and the sill keep her facing what she's at; if the
strip should too, `ReadStrip` moves to the non-turning arm of
`Pose::turns` (and `look_up`'s borrow arm loses its turn-back). Left
for the user.

**Why the slide back is a reel run backwards** (M8: "name the
slide-back"): `LayerOp::Unreel { step }` leaves the layer exactly as
`Reel { step }` did, farthest glyph first, so the strip goes back the
way it came, a cell a step, the line whole at step 0. An instant mend
would read as a blink of text reappearing; the reel in already shows
that the motion is cheap in attention (all in place, beside her).

**Why the strip is static art:** `ReadStrip` holds the approved strip
part (two fixed text colours). Drawing it from the real torn glyphs is
a wiring option the art left open; not taken: it needs a procedural
part per frame, for a strip a few cells wide that reads as text either
way.

**Why a key press catches her out tearing chat text:** local input puts
back what she moved in the chat and catches her out if she was at it;
`shaken` knew pulls and swaps only, so a resident tearing chat text for
a piece had what she held put back under her and tore it straight off
again (the goldens' resident runs, at a key press about 143 s in). The
borrow would have joined it; both are now caught out.

## Her window hangs low for her to lean on (2026-10-06)

**Rule:** The window's bottom row is 2 rows above the floor (it was 4),
its sill at her chest. It may share cells with a sofa (drawn behind it),
only while she could still lean on its sill from an end clear of the
sofa (the sofa covers a corner of it, never its middle), and with
nothing else that stands, nor with a sofa still in its box: one
predicate, `Shown::may_overlap` (on the pieces as they stand, through
`Furniture::may_overlap` for the pair), says so, and every placement
reads it both ways round. See [design.md](design.md#houseguest).

**Why:** the user's call on the art (phase 5c, round 2): "I think it has
to be lower. The window being drawn behind the sofa would look fine from
a design perspective, unlike other furniture." Leaning on the sill, chin
in her hands (`SillLean`), is the daydream the user asked the window to
be; at 4 rows up she could only gaze up at it.

**Why only its corner** (review of step 11): a first cut let the window
share any cells with a sofa, so a 4-wide window could hang wholly inside
the 9-wide sofa's top two rows: hidden, and out of her reach, as any
older home whose window hung over its sofa (natural at the old height)
loaded. The user's words were for the sofa covering the window's
corner ("as on the sheet": two of its columns), so the predicate asks
what makes the corner a corner: an end of the window she can lean at
clear of the sofa. That ties the overlap to its one purpose, so no
placement can make a window she can't see or reach for the sofa's sake.
A parcel is a box, not a sofa: the window shares nothing with it, so a
sofa never comes in under her window (she can move it there).

**Why one predicate, read everywhere a piece goes** (the class, not the
report): the wall lane used to be packed on its own, which was safe
only while every hung piece cleared the tallest standing one. Now:
- the cell test every placement uses (`room::free`: a frame's layout, a
  stage gift's spot, a delivery's admission) takes the piece being
  placed, and lets it share cells only with what it may overlap; her box
  (room to use a piece) may share none, so it's a separate reading
  (`None`), never the piece's;
- the layout (`Home::laid_on`) hangs a hung piece that meets a standing
  one it may not overlap at the nearest place between its neighbours on
  the wall that meets none, in order, preferring a place she could lean
  at it from an end clear of what stands (nearest first, so a window
  shifted off her TV doesn't land squeezed between the TV and her lamp
  when the wall has room past the lamp), else in the closet alone. So
  no frame can show the window over her TV, or hidden behind her sofa,
  however the home came to be: an older home's window, saved at the
  old height over a piece that stands, loads unchanged and shows beside
  it, or with only its corner behind the sofa (nothing is rewritten:
  the record has no hang);
- putting her home right (`rules::evaluate`) refuses a move that would
  push a hung piece further from where its wall alone would hang it
  (`Home::hung_shifts`: a piece set down under the window), or that
  sets the moved piece itself where it can't hang; a move that lets a
  pushed window back (her TV moved off its place) is a repair like any
  other (a first cut refused any move after which a hung piece lay
  elsewhere, so she'd move her bed rather than the TV under her
  window). Its frame test reads the same predicate for the piece and
  none for her box;
- a delivery is refused at a wall where the parcel's piece would meet
  the window, or the window would meet a standing piece (the sofa
  aside), through the same cell test.

**Why drawn as one image with the sofa:** in line art two images over
the same cells each hide the other's cells behind their placeholders, so
pieces whose footprints meet are painted as one image (back to front:
each hung piece behind a standing one first), as her box and her door
already take in what they overlap, in the same order. The one image's
box takes in cells that are neither's (under the window's free end,
beside the sofa's): if text is there, it isn't the image's to derez, and
the two are drawn each alone. In ASCII the sofa's glyphs are drawn after
the window's, and what she painted over what (the dissolve's cells) is
read off the screen before either is drawn, one cell each, the front
piece's: otherwise the sofa's cells over the window would "rain back"
to the window's glyph. Nothing else ever overlapped, so both orders are
unchanged for every other home (every golden trace byte-identical).

**Why she leans from just outside an end** (amended in step 8c: facing
right she now stands a column further in, as the sheet has her; see "Her
stillness ships", below): her face over the glass,
her box takes in the window's end columns (her own window's cells are
exempt from the room she needs, `clear_of`'s `of`), and a sofa the
window hangs behind blocks that end, so she leans from the free one
(the overlap rule keeps one free). The two ends mirror each other (her
box centred a column outside either end): the approved sheet's facing-
left lean is exactly that, and her box mirrors about its middle column,
so facing right is its mirror image (the sheet's facing-right figure
stood a column further in, which would put her box over a sofa covering
the window's other corner).

**Sibling, made pieces:** a makeshift piece must stay clear of her real
pieces (`tend_made`) but was offered wherever the text and her room
allowed (`builds` didn't look at real pieces), so one made under the low
window would fall apart as it was made. Both now read one predicate,
`clear_of_real`. (A census and a probe found no such build on today's
screens; the shared predicate makes the mismatch impossible rather
than tested.)

## Her window is a long daydream (2026-10-06)

**Rule:** Looking out is rarer (base 8 → 3; 6 in step 8c, 4 in step 12c) and much longer (60–180 s,
lingered by mood). It's a session: the sky's line, then leaning on the
sill with up to three musings on the sky (a whim's count), a whim's gap
apart, each from the sky as it is when she says it, waiting while she
speaks or looks up at the chat. She settles in from it: sitting in
front of the window, then dozing there or lying back watching the
clouds. Watching the clouds is only ever under her window, eyes open,
one line at most, and no doze. See [design.md](design.md#houseguest).

**Why:** the user's call (phase 5c brief): "the window is a daydreaming
place", not a quick glance; 5b's census had about 19 short look-outs a
game day in a furnished home. Movement draws the eye; one long lean
replaces many walks to the window and away.

**Why musings said as they come, not drawn at the start** (M10 allowed
either): three minutes of real time is 18 of her clock, so the sky can
turn mid-look (17:00, 19:00, 21:00); a line drawn when she leaned would
say "Sunny!" at dusk. Each musing is drawn from the session's whims (a
series per musing), so a decision says the same however her stream runs,
and the session lives on `Osaka::sill` beside her act, cleared with it
in `set` (no new field on every `Act::Use`). The pools are one per sky
(at least four lines each, all five skies), cooling as any line does.

**Why up to three by her mood's lever:** how many musings she has is
the stillness band's to tune, as a daydream's are, so it's a lever of
its own (`Stillness::sill`, from–to by mood, three at most), not B6's
daydream count (which ships at one: the sill would always say one).
`NEUTRAL` ships it as built, none to three by whim in every mood;
`STARTING` follows the daydreams' spread. Its length lingers through the
length she draws, so the band's guard sees it (`longest_still_ms`:
180 s, within the band's 9 minutes; with lingering and settling on, the
sill's chain sets the visit length, see baseline.md).

**Why sitting in front of it doesn't turn her:** her face is up at its
sky, chin in her hands, as aimed at the window as the lean is, so a
chat line has her look up in place (`Pose::turns`). Lying back to watch
the clouds takes its facing from the look-out seat she sits at, not
from however she was last turned, so her head is under the glass.

**Why cloud-watching is a pose of its own** (`Pose::CloudWatch`): a doze
is read from the pose drawn (`Pose::dozes`: a chat line stirs a doze,
"Mm?", and looks up at anything else). Cloud-watching is drawn as lying
back with her eyes open, and must look up, so it can't be `LieBack`. It
draws as `LieBack(0)` (the art lint's one exemption beside it), held,
curious, her slow blink on.

**Why only under her window, structurally:** it's settled into only from
sitting in front of the window (`UnderSill`), which is settled into only
from leaning on its sill, and only while the window's look-out seat is
still where she sits; else she dozes. A sit on a bare floor still lies
back to doze. Lying down she turns end to end from how she sat, so her
head is under the glass.

**Shipped inert, as every settle-in did:** settling's odds were the
band's tuning, all 0 until it (`Stillness::NEUTRAL`). The sill's session,
its length and rarity shipped then; sitting in front of the window and
watching the clouds came with settling on (step 8c, "Her stillness
ships", below), with base 6 and the glance's roll gone.

**Measured (the day census, 6 runs a room, a game week each):** in the
census home with a window, 14 look-outs in the 42 game days (683 before:
about 16 a day, now one in three days); at base 6 it would be 82 (about
two a day, the "few a game day" the design aimed at). With the corner
rule (review) the census home's window, given wherever a gift lands, is
never hidden behind its sofa: 35 look-outs in 42 days (0.8 a day). Her
window is offered (in reach) at about 4200 of her decisions' steps, in
her top four at 172, chosen at 35: it's rare by its score (base 3), not
by reach. Base 3 is the design's number, kept: the rest is the user's
call. Her afternoon glance at her clock is still starved: 1–2
"Three-ish." in the home's 42 days (4 before), 2 on the stage, 9 in the
resident's. The design's fallback, read as lifting the glance's once a
visit so it rolls once per daydream session (it rolls only on a
session's first musing, B6), was measured: the same 1 / 2 / 9, since
the once a visit never binds (she glances far less than once a visit).
So it's not built: the starving is in the glance's other gates (her
clock on the strip she stands on, of an afternoon, quiet, a musing in
three, on a daydream's start), and which to ease is the user's call.

**Why base 4 (the user, step 12c: "about two long window daydreams a
game day"):** the user chose the outcome, not the number. Base 6, the
user's number in step 8c, came to about two a day with the levers
neutral, but with them on (lingering, settling in) the day census read
4.5 a day. The mapping is steep, since a look-out has to make her top
four against her other daydreams: in the census home's 42 game days
(the driver painting at each chat line, an industrious watch whole),
base 3 gave 36 look-outs, 4 gave 82, 5 gave 140, 6 gave 175. Base 4 is
the nearest two a day (1.95).

**The tolerance (the user, after base 4 had been measured):** "4.5 times
a game day is fine. I'd accept anything in the 2-6 range." Base 4 stayed
(1.95, two to the user's rounding) until phase 5c's tail, below. A
deliberate retune within two to six a day needs no new sign-off.

**Why base 5 (phase 5c's tail, T1, 2026-10-07):** base 4's 1.95 a game
day sits just under the 2 to 6 the user accepts; base 5 measured 140
look-outs in the day census's 42 game days (3.3 a day), mid-range, so
neither noise nor a neighbouring change pushes it out. The day census
now holds the user's range itself, 2 to 6 a game day in a room with her
window, not a band around what ships: it fails only outside that range,
so drift inside it shows only in the number the census prints. No band
room has a window, so no band cell moved; in the fed afternoon's
windowed home (printed, judged by nothing, 15 minutes against the
band's 17) her time leaning at the sill rose from at most 3% of an
afternoon visit to 1.5–12% by mood (of an evening, 6–17% to 13–23%),
moving in sight moved by at most 1.2 points, mostly down, and every
cell stays inside its mood's band by eye. Base 4 never
reached players (all of 5c was unpushed), so the changelog's "she looks
out of her window more often" (against the pushed base 3, 0.33 a day)
stays true and needs no new entry.

## Her stillness ships (2026-10-07)

**Rule:** The stillness levers ship (`Stillness::TUNED`): lingering
(lazy ×1.5, ordinary ×1, dreamy and industrious ×0.85; an industrious
watch ×1, since step 12c), settling in
(lazy 0.6, ordinary and dreamy 0.35, industrious 0.15; from sitting,
dozing where she sits half the time), daydream musings by mood, homework's
nod-off by mood, nearer spots; with step 8's lengths and needs (8b's run
c3): restless rises over 5 minutes, tidy over 90 s, mischief over 6; Walk
base 9, Travel 7; spacing out is worth doing for its own sake, 10–28 s; a
chosen stand 3–8 s; sitting 15–40 s, lying back and dozing (sitting up
or under a book) 20–60, gazing 6–14, lounging 27–60, napping 45–105,
watching 32–82, reading 30–65, homework 37–75. Sitting in front of her
window lasts as sitting does, watching the clouds as lying back does.
Looking out's base is 6 (4 since step 12c). Of an afternoon, the first daydream she starts
quiet with her clock in view glances at the hour (once a visit). Reading
a borrowed strip she looks up at chat without turning. The band's floors
on the bare stage are lower (lazy 4, ordinary and dreamy 6, industrious
9). See [design.md](design.md#houseguest).

**Why lower floors on the stage (the user's call, 2026-10-06: "ship it,
lower stage floors"):** a room that keeps her things close (the pieces
she made, the lines on her own floor) moves her in shorter trips, two to
four seconds a set-off against eight to eleven in a furnished home. At
the same set-off rate its share runs lower, and under a per-mood cap on
set-offs it can't reach the furnished floors without her setting off
more often, which is what draws the eye. So the stage's floor says what
"not dead" means there; its ceilings and caps are the same.

**Why dreamy lingers less than ordinary (0.85):** with settling on, the
reading on her back of 10a and the sill chain of 11, a dreamy afternoon
where her text is above her (the band's resident) fell under its floor
(8.6 / 7.0 at linger 1). A dreamy Osaka is still by her daydreams (two to
four musings a session, sessions as long as they last), so lingering a
little less over her other still acts keeps her moods' ordering and
brings her in (9.2 / 8.6); the home's dreamy afternoon moves from 20 to
21. Lazy lingering more (2.0) would have taken the resident's lazy cell
under its floor; it stays 1.5.

**What shipped short, by the user's word ("if one cell can't be brought
in, ship the closest"):** the bare stage's industrious afternoon with chat
(8.4% at full strength, its floor 9; quiet 9.1, in) and the stage's
spread (industrious ÷ lazy 1.39 against 1.6): step 8c's numbers. What
shipped is step 12c's, 8.6% and 1.38 (phase5c/baseline.md, "Census pass
(step 13)"). An industrious linger of
0.7 (the design's start) brings the stage's chat cell to 9.2, but the
furnished home's industrious afternoon to 32.4 against its ceiling of 31,
the old conflict between the two rooms; Tidy back to 60 s moved the stage
within noise again. On the stage her moods read alike: every mood reads on
her back about a sixth of the visit there, and an industrious Osaka,
whose needs fill slowest, spaces out most (14% of the visit, ordinary
7.5%): spacing out for its own sake weighs the same in every mood, so
where little else calls her it fills an industrious afternoon most, the
lead for the stage's spread (a mood factor on it). The chat cell's test
stays ignored with its numbers (`tests/band.rs`, `SHORT`; its quiet cell
is in the gate), and the spread's, for the user to weigh.

**Why base 6 for looking out (the user's answer):** measured with the
levers neutral, about two long daydreams at the window a game day in a
furnished home, the design's "a few a game day"; base 3 gave one in
three days. With the levers on, the day census's home looks out about
4.5 times a game day (191 in 42), each look lingered by her mood: more
than the user expected, left to the user (phase5c/baseline.md, "Shipped
(step 8c)").

**Why the hour glance lost its roll (the user's answer):** her daydreams
are long and few now, so one in three of their starts almost never came
(1–2 a week in the census home; now 19). Every other gate stays: an
afternoon, her clock on the strip she stands on, quiet, a daydream's
start, once a visit.

**Why a borrowed strip doesn't turn her:** she reads it beside its tear,
facing the line; turned, she'd hold the strip out over the air, away
from where it came from, as at her desk or the sill (`Pose::turns`).

**Why the facing-right lean moved a column in:** rendered at the spot
the code used, her hands met the window's frame where the facing-left
lean has them on the sill: the lean's figure sits a column toward her
back in its box, so the two ends aren't mirror images in cells. The
approved sheet stands her facing right with her box centred on the
window's first column, and the code now does too (`look_out_spots`).
The overlap rule with a sofa reads the same spots, so a sofa hiding the
window's right corner now blocks a column more of its left end.

**Why `BAND_MINUTES` is 17:** the band's guard asks for the warm-up and
twice her longest still act; with lingering and settling on, the sill's
chain is longest (a lazy lean of 270 s, 60 s sitting in front, 90 s
watching the clouds: 7 minutes).

**Why the golden driver cuts at its events (landed alone, before):** a
golden step's length is set by her wakes, so with delivery at the first
step past an event a wake that only changes how she looks moved what she
did, and a trace diff read further than the change. The census drivers
already cut there (step 8b). A key press or a cue painted at its time;
a chat line or a focus change came into her view then but she saw it at
her next paint, which her wakes time.

**Why the drivers paint at every event (phase 5c step 12c, landed alone,
before the user's answers):** the client draws after every input
(`ui::shell`'s loop: a chat line, text arriving, a key press, a focus
change), so she sees a chat line the moment it comes; the drivers showed
it her at her next wake, up to 10 s later for the resident, so the
censuses measured a slower Osaka than the client runs. The golden driver
now paints at every event, the census drivers (visits, fed afternoons,
the band, the day census) and the film tests' at every line. One
re-record, and every chat cell re-measured (phase5c/baseline.md, "Step
12c").

**Why the golden driver advances her before it tells her of input (phase
5c's tail, T1, 2026-10-07; test-only):** the shell advances her as it
takes each input, then handles it, then draws, so whatever fell due by
a moment has happened when a key press comes then. The driver had told
her of a key press first, so a key press the moment school ends found
the client busy (her coming home is asked once, as it's due, A6). Its
cues follow the same order by convention, not to match a client: the
shell has no cues, and the stage cues on a key while her clock stands
at its last draw, so a cue on the very moment something falls due (she
goes out by her door) is a tie there; advanced first, it finds her gone
and brings her in. The two property drivers that press keys
(`a_resident_keeps_out_of_the_focused_pane`,
`errands_end_and_touch_only_the_accordion`) took the same order. Neither
moment comes up in a golden scene, so no hash moved;
the ~10 Hz snapshot redraws during playback stay unmodelled (a cadence
the session sets, not hers; modelling it would multiply the errand
scenes' paints tenfold and move them for the harness alone).

**Why watching is longer though it doesn't linger (32–82 s, was 20–45):**
step 8's lengths, which 8b's run c3 carried and the user shipped,
lengthen every still use alike, watching with them, so a visit's share
of watching keeps its place among her uses. Lingering is another thing:
it would stretch only a lazy watch, half as long again, and watching
stays out of it until her TV holds a picture (phase 5c D7, step 12a)
rather than animated static.

**Sleepiness alone no longer lays her on the floor:** spacing out, worth
doing for its own sake now, outweighs a floor doze that only her
sleepiness asks for; with comfort wanting too, the doze is still her pick
there (`sleepiness_draws_her_to_lie_down`). Over a visit she still dozes
more as she tires: she gets uncomfortable as well as sleepy.

**The chat's tenth governs going in, not staying:** a resident's offers
that would take her into the chat are a tenth as likely, but what she
does where she stands (spacing out, an on-the-spot act, settling in from
it) has no chat weight. With the levers on, once in the chat she stays
longer, as she does anywhere: a resident's share of her time there rose
from about 5% to 9–13% (a visitor's 16–21%, from 25%;
`a_resident_mostly_keeps_out_of_the_chat` pins it under 0.7 of a
visitor's). Weighing her staying too (the on-the-spot wants and settling
paying the tenth while she stands in the chat) is the user's call: it
would have her walk out of the chat more, so it moves every resident
cell of the band and asks for a re-measure.

**The settle-ins under the window sit where she leaned:** sitting in front
of the window and dozing there keep her lean's spot, so on the approved
sheet's terms they sit a column further out facing right (her face at
the window's first column, not its second) and, facing left, beside its
end rather than under its last columns; watching the clouds, turned end
to end, has her head under the glass either way. Settling in is in
place, with no step, and a spot further in would need its own clearance
(a sofa covers the window's other corner); the difference is accepted
(phase5c/art/snippets.md).

## Her TV holds a picture (2026-10-07)

**Rule:** Her TV shows static only as she switches it on (`STATIC_MS`,
1.2 s: three of its 400 ms frames) and between channels as she flicks;
then a drawn programme (the news, the weather, penguins, a cooking show)
holds still to the end of the act. The programme is drawn from her
decision's whims as every watch begins, whatever it plays (a plain watch,
the shopping channel, a surf, a watch with her home on her mind) and the
act carries it (`Play::card`). Chiyo-chichi bobs only through his hook
(the shopping channel's first two fifths), then holds still. Watching
lingers by her mood as her other still uses do, but an industrious watch
isn't shortened (`Stillness::watch`, step 12c). A script key names what
it shows as `Shows`: held as it is, static, the hook, or the act's
programme; only static and the hook move. See
[design.md](design.md#houseguest).

**Why:** the TV is the one thing in a TV home that moved all the time:
static for the whole of a 32–82 s watch, reshuffled on every paint, and
on the shopping channel Chiyo-chichi bobbing for its whole length. In a
phase about her being still, the brightest flicker on the screen was her
TV. A held picture after a short switch-on reads as "she put something
on", and the change she causes (switching on, flicking) explains itself.

**Why the card is drawn on every watch, from the whims:** step 12b will
show a frame of the film instead of the card when this client holds it.
Whether that frame arrived depends on mpv's timing, so nothing she does
may depend on it: the card is drawn whether or not a film will cover it,
from a labelled whim (`Whims::below("programme", 4)`), which draws
nothing from her generator. What she does is then the same with or
without the card, and the same with or without a film
(`every_watch_draws_its_programme`).

**Why a fixed `STATIC_MS`, not "static until the picture":** waiting on
the picture would put mpv's latency into her trace. 1.2 s is three of
the static's frames and ends on a frame's edge.

**Why `Shows` rather than a held `Channel`:** a script is a `const` table
and can't carry the act's drawn card, and a `Channel` is the TV image's
cache key, which should only ever name something drawable. So a key says
what it shows (`Shows::Programme`, as `Say::Pitch` says what she pitches)
and `Osaka::prop` resolves it with the act's card to a plain
`Prop::Tv(Channel::Programme(card))`; what moves (static, the hook) is a
property of the key, so Chiyo-chichi after his hook is the same image as
his first frame, held.

**Why watching lingers now:** it was held out of lingering only because a
lazy day would have watched animated static half as long again (step 8b).
With the picture held, a long watch is a still one. Measured
(phase5c/baseline.md, "Step 12a"), it moves only the lazy, dreamy and
industrious cells of the rooms with a TV. The furnished home's industrious
afternoon, quiet, now reads 31.4% at full strength against its ceiling of
31 (step 8c: 30.6). Its test passes within its 3σ, but the cell is over.
Its shorter watches (×0.85) get her up more often.

**Why an industrious watch isn't shortened (the user, step 12c: "Don't
shorten industrious watching"):** an industrious Osaka shortened at the
TV got up from it sooner and so moved more, pushing the home's
industrious quiet afternoon to 31.4% against its 31% ceiling. Her other
still uses keep the industrious ×0.85 (it's what keeps her busy), and
every other mood's watch lingers as that mood does. The watch's table
(`Stillness::watch`) overrides the linger only where it says (×1
industrious), not a second whole table: so nothing else she does moves,
and a later change to the linger still reaches her watching unless
the user said otherwise (step 12c's review: two whole tables would let a
lever over "linger" silently miss the TV).
Measured in phase5c/baseline.md, "Step 12c".

**The stillness test** (`no_long_act_flips_faster_than_a_frame`,
`a_long_tv_act_holds_still_after_its_first_ten_seconds`): sampled every
100 ms, as a client painting for any reason shows her, no act that holds
her place for over 30 s changes how she or her furniture looks twice
within a frame (1.4 s) after its first 10 s; her slow blink, the
hook's bob and a look up at the chat are exempt. A walk the screen's
width is over 30 s and changes every step, so an act that moves her is
outside it: what moves her is the band's to count (a turn where she
stands is checked). A chat line's look up (`!`, then `?`) is checked
elsewhere, so the test runs quiet. It runs every mood, in the three TV
rooms, the home with her window (its sill session is her longest still
act) and the furnished home with the shopping channel on (so the hook's
exemption is tried, not only the script's own test). It compares her
model's state (how she looks, where, what her script shows, the lamp),
not drawn cells: a proxy for what's drawn of her and what her script
shows, which map from that state (`each_prop_looks_distinct_in_each_mode`,
`every_piece_shows_what_her_script_shows_on_it`). Some of what's drawn
isn't in her model: step 12b's film, her wall clock's dial (each game
quarter-hour) and her window's sky. So the rule has a drawn variant
(step 12c, `no_long_act_flips_drawn_cells_faster_than_a_frame`): painted
every 100 ms as a client would paint (advanced first, as the shell
does), with chat, in the TV-only home, the home with the shopping
channel on, the home with her window and the resident, every mood, both
modes, the film's stills fed through `TvFeed` as the shell feeds them,
it compares the cells and images painted. Its exemptions are design.md's
five (the world's clock since phase 5c's tail, T1, below), each only
that change with nothing else drawn changing alongside
(step 12c's review: a looser check let any drawn change ride along a
look up): a blink changes cells only in her box and no look but hers;
the hook's bob only the TV's cells and look; a look up (or a stir)
changes her pose, face, bubble or facing in her model, no look but hers
and no piece's cells outside her box; the film's fresh still, at a paint
a still was delivered at, takes the TV from a still (or the programme
it stands in for) to another still of the same programme, and nothing
else (that it comes once a minute is
`a_long_watch_takes_a_fresh_still_once_a_minute`'s to hold); the
world's clock, at a paint where a reading of her clock changed (the
dial's quarter-hour, the sky) and, in line art, that piece's look with
it, her model, her key and every look but the dial's and the sky's the
same, every changed cell in the footprint of a clock or window whose
reading changed and none in her box. Mutants
it fails: one that swaps back to the still before at every paint (the
TV's image flipping 100 ms apart, no still delivered; the model's test
can't see it), and one that flickers a cell above her every 200 ms while
she looks up at the chat (the looser check passed it).

**Open from the drawn variant (step 12c, the user's call):** with chat
on, a chat line stirring her asleep by day ("Mm?") showed up as a
change just after her breathing's flip. A stir is the dozing form of a
look up at the chat, so the test exempts it with the look up; but by
day it lasts `speech_ms("Mm?")` = 1380 ms, under a frame, so it comes
and goes inside one (a night's "mm..." lasts 1500 ms). Making it last at
least a frame (`speech_ms(line).max(USE_FRAME_MS)`) would change how
she looks (a golden re-record; since T2 it lasts a frame: below). And in the home with her wall clock its
dial steps within a frame of her own changes (her breathing asleep:
656 ms apart), as the window's sky would at dawn and dusk: the world's
clock is no act of hers, and steady and slow, so whether the rule holds
it is the user's call; its run is kept, ignored, with those numbers
(`no_long_act_flips_drawn_cells_faster_than_a_frame_by_her_clock`;
since T1 exempt, and in the gate: below).

**Why a stir lasts a frame, and stays exempt (the user, phase 5c's
tail, T2, 2026-10-07: "fix both"):** the stir's turn lasts as long as
its murmur, and the murmur as long as anything she says, so the floor
is on how long she says a line (`speech_ms`: 1.2 s + 60 ms a
character, a frame at least), not on the stir alone: stretching only
`stir_until` would have left "Mm?" vanishing at 1380 ms and her turning
back 20 ms later, two changes in place of one. The floor reaches every
line, so none can come and go inside a frame; today only "Mm?" (three
characters) was under one (`whatever_she_says_shows_for_a_frame`). A
stir stays exempt as the dozing form of a look up: it comes when the
line comes, off her breathing's frames (without the exemption the drawn
test fails at once, a stir 956 ms after her breathing's flip, every
room), as a look up does; what the drawn test now checks is that it
starts with a line and, at every paint of the frame from its start,
shows as it began (her blink, its murmur, her turn; what she's turned
over from may change under it, as her key does). Her look up's `!` (1.2 s,
`SURPRISED_MS`) is under a frame too; it is left: it is inside the
look up's own exemption, a reaction to the user's line, and the same
startle she gives on her feet, so changing it is a design call of its
own (open, plan.md). The other things of hers under a frame are not
lines and are out of the rule's reach: the andagi's quiet beat (0.6 s)
and the no-melon look back in the fridge (1.2 s) are in codas to a
snack of 6–9 s, the whole under 30 s; the chopsticks' split (0.8 s) is a
prelude, inside a use's first 10 s; Setsubun's throws (0.7 s each) are
an act of 5.6 s; a glance at something lost (0.9 s) and peering over an
edge (1.2 s) are short acts of their own. Golden traces: only each
daytime stir's end moved, from 1380 to 1400 ms after it began.

**Why the world's clock is exempt (the user, phase 5c's tail, T1,
2026-10-07: "exempt it"):** her wall clock's dial and her window's sky
are not her doing, and they step slow and steady (each game
quarter-hour, about 150 s at 6×; the sky five times a game day), the
periodic motion the eye filters out (the principle the hook's bob
rests on); a change the eye catches is hers. The exemption has a
trigger, as each of the others has (a blink a face change, the bob his
hook, a look up the chat, a swap a delivered still): a reading of her
clock changed between the two paints (the dial's quarter-hour, or the
sky), and in line art the look of a piece whose reading changed went
with it; without one, a cell flickering in the clock's footprint at any
speed would pass. And it is scoped to that change alone, so it can't
carry hers: at that paint her model and her script's key are the same,
no look changes but the dial's and the sky's (each only if its reading
did), and every changed cell is in the footprint of a clock or window
whose reading changed and none in her box. The by-her-clock run is in
the gate now, of an afternoon (four dial steps) and from 16:30 across
the sky's day to dusk; each run sees a dial step in a long act, the
dusk run a sky step, and at every paint the exemption holds at, the run
checks that the same paint with one thing more changed is not exempt
(no reading changed; a cell of her box; a cell outside every footprint;
her model; another look). Mutants it fails, both modes: a cell of hers
changing with the dial (her model the same) and a cell outside the
clock and the window changing with it, each in both rooms (the
afternoon's at the dial step 656 ms after her breathing's flip, the
dusk room's at one 38 ms after it); and a cell of the clock's, or of
the window's, footprint flickering every 200 ms, off the quarter, at
its first flicker in a long act. Before the trigger the flicker passed
as the clock's for over a minute and was caught only where it spoiled
an exemption of hers.

**Why a bob keeps a frame clear of its key's start and end (the user,
phase 5c's tail, T2, 2026-10-07: "fix both"; step 12c had allowed it
provisionally):** a key that bobs (her writing, her breathing asleep)
flips on the 1.4 s frame grid of its part, and a key ending at a share
of the use (her homework nodding off) lands off that grid, so it came
less than a frame after the bob's last flip: two changes within a
frame. Step 12c's test allowed exactly that case; the user chose to fix
what she does instead. So a bob holds its last frame through the part
of a period before its key ends, and its first until the first flip on
the grid a whole period after its key starts (`script::bob_frame`): its
flips and the changes as its key starts and ends are always at least a
frame apart, wherever a share puts them. It stays on its part's grid
(not restarted at each key) because her wakeups are on that grid
(`every_bob_moves_on_the_frame_grid`, the wakeup tests): a held flip is
a wake with nothing new to paint, so nothing she does moves, only how
she looks. It starts on the grid's frame at its key's start, so a key
starting on the grid (every part's first) is the plain beat but at its
end; after a held first flip its beat runs the other way, which nobody
can see. One mechanism covers every bobbing key, all on the frame
(`Key::look` takes the key's `KeyTime`, so no bob is timed without its
key's edges): napping and asleep (by day, the night's bed and sofa
branches, the Dream's), her homework writing at her desk and on the
floor (and reading there), reading at her bookshelf, eating the melon
bread, and crumpling and unpacking (bent to it). Its siblings outside
scripts hold their last frame before the act ends the same way: lying
back and reading on her back (idle acts bobbing on the frame, which
start on their grid) and reading a borrowed strip. The hold is chosen
by period, not by the act's length: the user asked for every bobbing
key, so it reaches bobs on the frame in short acts too (crumpling and
unpacking, the melon bread), where the stillness rule itself doesn't.
Faster alternation (exercise, kicking her feet, lifting and setting
down a piece, poking, carrying a parcel home, static and Chiyo-chichi's
hook, all quicker than a frame) is motion, not a still act's change,
and is left to run to its end: holding a frame at each end of a key of
700 ms flips would stop most of them, and none is in a long act the
rule reaches (each is in an act under 30 s, or exempt).

The cost is flips in short bobbing keys: a key can only flip on the
grid points a frame clear of both its ends, so one under two frames
never flips, and one of two frames flips once only if its middle is on
the grid. Crumpling and unpacking (keys a share of a 4–6 s use) bob
once or twice where they bobbed three or four times; her breathing as
the lamp goes off was a 2 s bob and is now drawn still (`LAMP_ON_MS`,
it could never flip); the andagi's chewing, two frames off the grid,
never flipped at all (T2's review), so it is two keys now, chewing
then biting again, a frame each, as the vignette was approved ("two
frames"); and the Dream's second and third lines (3 s each, starting
off the grid) hold her breathing still, one flip in its 9 s where there
were six. The Dream is left for the user (two frames a line, 2.8 s,
would give each line a flip; plan.md; resolved in T4: below). `every_bob_at_a_set_time_flips`
holds every bobbing key that plays at set moments to at least one flip,
the Dream's two lines excepted by name (no longer: below). And where her act's end is
retimed in place within its last frame, the hold was timed from the old
end, so her breathing can flip then, off the grid, and its next flip
come within a frame: a night whose wake time moves (`refresh_night`),
and a day's sleep or a night's idle act becoming her night at bedtime
(`sleep_on`, which also drops any coda). Both need the retiming inside
a frame of the old end; left, and open (plan.md; resolved in T4:
below). Golden traces: 75
moved, each first at a bob's frame held within a frame of its key's or
act's end (crumpling or unpacking 42, asleep 14, reading on her back 9,
homework 6, lying back 2, napping 2), every differing line only a pose
frame, no time or act changed. The stillness tests hold the rule
plainly now (no allowance), and the model's test adds an industrious
afternoon at her desk (seed 0), where the writing's end comes 800 ms
after its last flip without the hold.

**The Dream's lines are three frames each (the user, 2026-10-08):** of
the two lengths that give each line flips (two frames, 2.8 s, a flip at
its middle; three, 4.2 s, two), the user chose 4.2 s. Each line starts
on her breathing's frame grid (the Dream's part starts its own grid),
so each flips twice, six flips in its 12.6 s, as many as the 3 s lines
had before the hold; the Dream asks for 12.6 s of her night left, not
9 s. `every_bob_at_a_set_time_flips` checks the Dream's lines with every
other bob at a set time, no exception by name; it found no other set-time
bob that the hold leaves without a flip (the Dream's are the only
bobbing keys set in ms; the share-timed ones, crumpling and unpacking,
are above).

**A bob holds where her act's end moves in place (phase 5c's tail, T4,
2026-10-08; T2 left it open):** a bob's hold before its end is timed
from the end known then, so an end moved within its last frame released
(moved later) or took back (moved earlier) a flip at that moment, off
the grid, with the next flip or the act's end less than a frame on.
Two places move an end in place: `sleep_on` (a day's sleep, or a
night's idle act, becoming her night at bedtime; it also drops any
coda) and `refresh_night` (her wake time found afresh at each reading
of her clock, which now gives the moment it's read at). Both now hold
her bob (`Osaka::hold_bob`, before the end moves): the frame it shows
then holds until the first flip on its grid a frame on
(`script::bob_frame`, the hold carried in `KeyTime` to every bob
on the frame: a key's, lying back and reading on her back, reading a
borrowed strip). The rejected alternative, restarting the bob at the
moment as at a key's start, would show the grid's frame there, not the
one shown, and change it off the grid itself. A hold belongs to its
act (cleared as an act is set) and to its part: the Dream, which
restarts her night's part in place, carries none, and keeps clear of
its own start as any key does. Where the end moves earlier to less than
a frame on, the act's end itself can still come within a frame of the
last flip, which no hold can take back; it is her waking (a new act),
and only a date crossing into a vacation (or back) in the night moves
the wake at all. A hold never moves back (T4's review): a tick that
comes late reads her clock at its own moment first and holds there as
her wake moves, then handles what fell due before it, and a line's end
held at its earlier moment replaced the later hold, so her breathing
could flip less than a frame after the moment the client painted from.
The latest hold now stays (`Osaka::hold_frame`); and `refresh_night`
reads the frame before the end moves but holds only if it moved (it
held and put the old hold back at every reading, logging a hold that
never was each tick). Test: `a_late_tick_holds_her_bob_from_her_wake_moving`
(lines ending in each 100 ms of the frame before a late tick, three
phases of the grid), red before ("held from her wake moving": 899900,
not 900000; her pose "changed at 900000 and 900600");
`any_held_bob_frame_holds_from_its_moment` also tries a hold from
before the bob's start, which holds nothing.
Test: `a_bob_holds_a_frame_when_her_acts_end_moves_in_place`
(every 50 ms of the old end's last frame, three phases of the grid, both
places, both ways), red before the fix at `sleep_on` ("changed at
300000 and 300800") and, the hold dropped from `refresh_night` alone,
at either way the wake moves. Golden traces: none moved (no golden
scene moves an end in place).

**An exempt change of hers still counts, and her bob holds from it
(phase 5c's tail, T4, 2026-10-08):** the stillness rule exempts some
changes, "each only its own change", and the drawn test had read that
as "never counted": an exempt change never set the frame clock, so a
look up at the chat or a stir ending off the grid was followed by her
breathing's next flip less than a frame later unnoticed (T2's review).
The rule is now precise: an exempt change is never a flip itself, but a
look up's or a stir's changes are her reaction, a change the eye
catches, and whatever of hers changes next waits a frame from them, as
from any change of hers. Her slow blink and the hook's bob are steady
periodic motion (the user's principle, Round 8), and the film's still
and the world's clock aren't hers and come on their own time; none of
those counts. Counting blinks was tried and rejected: her blinks come
off her bob's grid by design, so her reading or writing flipped within
a frame of a blink's end 120 times in the drawn test's runs; holding
her bob for each blink would all but stop it. The world's clock counted
would undo T1's exemption (the dial stepping 656 ms after her breathing's
flip is the same pair the other way round).

What she does now fits it: her bob holds the frame it shows from each
change of a look up (its beginning, its `?`, its plain watch, its end)
and from what she says coming, going or being cut short (a stir's
murmur is said, so its turn's end is held from too), until the first
flip on its grid a frame on (`Osaka::hold_bob`, the hold her act's end
moving in place already used). Before it the drawn test, counting on
from those, failed on a stir by day (the stir over at 181400 and her
breathing's next flip at 181844) and a look up over reading on her
back (its end at 94000, her page at 95267); 48 stirs and 2 look ups
in its runs. A hold skips a flip at most, so after one her bob's beat
runs the other way, which nobody can see. Golden traces: 67 moved,
each only her bob's frames (asleep, reading on her back, homework, a
nap, a borrowed strip, the paper desk), her acts, places, faces and
bubbles the same in every one.

T4's review tightened both. The drawn test's look-up exemption took
any change of hers while she looked up or stirred, her bob's flip and
her act's key included, so the holds at a look's start and steps were
pinned only by golden hashes: it now takes only the look's own change
(her face, bubble and facing; her pose only into or out of a stir's
turn, with its murmur), and anything else of hers under it is judged
as any change, so dropping either hold fails it, and so does its
mutant (her pose changed a paint after a look's change, the look still
on). Two unit tests sample her whole look every 10 ms through a look
up and through a line she says, in every still act and reading on her
back, at each 100 ms of her bob's frame: each fails with the hold at
the look's start, its steps, a line's start or a line's end dropped.
Two holds were reached by nothing and were redundant, so they went:
her look's end (each caller holds at the same moment, or a new key or
act starts its own grid) and a line cut short (every caller of `hush`
says, which holds, sets a new act, or begins the Dream's part). A
stir's turn now ends with its murmur when another line cuts it short
(her sleep-talk, the Dream, a line by day): it was woken for and held
at only through the murmur's end, so cut short it ran on to its old end
unwoken (`a_stirs_turn_ends_with_its_murmur_cut_short`, red before). The
same open class as her look's (above): a line she says a moment before
her act's key comes less than a frame before it (plan.md).

**A musing waits a frame from her look's end (the door batch, step 2,
2026-10-08):** her sky musings at the sill and her daydream's musings
waited for her look up at the chat to be over, but one due a moment
after it came less than a frame after its end: the door batch's push of
her pieces moved her afternoon enough for the drawn test to find one
("home, clock and window, to dusk Dreamy seed 2, an act from 503085
flipped at 545400 and 546718", her watch's end and "Orange, then
purple..."). Both now wait until a frame after a look's end, or say it
at the end itself, with the look's own change (`Osaka::free_to_muse`,
red first: `her_musing_at_the_sill_waits_a_frame_from_her_look_ending`
and the daydream test's case). *(Step 2's review, 2026-10-08: the case
seen once before that push, put down then to the shopping channel's
key boundary, came back when the review's gift fix moved her afternoon
("home, shopping Dreamy seed 2, an act from 50088 flipped at 95400 and
96460"), and it was a line: what her look hid, her pitch, said on at
94000 for 2460 ms, her look over at 95400, the line gone 1060 ms on. A
line still showing as her look ends now shows on until a frame past it
(`Osaka::end_look`; red first:
`a_line_never_ends_within_a_frame_of_her_look_ending`). Provisional, as
the musing's hold is.)* The class stays open where it hasn't been seen:
a scripted key boundary less than a frame after a look's step, and what
she says a moment after a line of hers ends (plan.md).

**The drawn exemptions combine, and the sky shows round her in ASCII
(phase 5c's tail, T4, 2026-10-08; T1's review left it):** the world's
clock's own step (its footprint's changed cells and its dial or sky
looks) is taken back first and whatever else changed at that paint is
judged by the other exemptions, so her blink as the dial steps is
exempt and her breathing's flip with it is not. In ASCII the sky's (or
the dial's) cells in her box, off her glyphs, are the piece's at a
paint where her model and key didn't change (her leaning at the sill
as the sky turns to dusk); her glyphs are known there (her sprite's
cells), and a change at one of them is hers. In line art she is one
image over the window, so her box is hers. The test's own mutants, at
every paint the world's clock exempts, show both: a blink with it is
exempt, and with a cell elsewhere as well is not; with her box put
over the stepped window, the sky's cells off her glyphs are the sky's
and one of her glyphs changing with it is hers. Each new clause was
shown to matter by breaking it in the test (the stripping, the glyph
check, the count from a look, the count from a flip), each failing its
mutant. Her blink and the hook's bob at one paint combine the same way
(her face, cells and looks taken as the blink, the rest as the hook's
bob), in both tests: in the model's test (her state, not cells) a blink
beginning on the sample the hook's bob flipped counted as a flip (T2's
probe: lazy seed 2, ordinary seed 5, which its shopping room now also
runs; with the combining broken it fails there, "flipped at 91900 and
92200"), and the drawn test's mutant (a blink with each real hook's bob
is exempt, with a cell elsewhere as well is not) fails with it broken.
The model's test counts the samples only the combined clause exempted
and requires some in the shopping room (11 in each mode), so a change
moving the coincidence away can't leave it untried unseen. Its limit,
documented in the test: those are the only pairs that combine (the
world's clock with any one); a look's change or the film's still on
the paint of a blink or the hook's bob is judged as hers, failing only
if something counted came in the frame before, which no run does. A
key boundary that shows nothing new is no change at a step of the
world's clock either, as at any paint. The film's still counts nothing
as design.md says, though counting it fails no run today: nothing keeps
her bob off the moment the player answers, so it would fail by chance,
on a principle that isn't hers to keep.

Left, and open: her night. The stillness tests run afternoons, and the
night's sleep is a long still act whose sleep-talk and the Dream come
at moments her breathing's grid doesn't know: each begins less than a
frame after her breathing's last flip, which no hold after it can take
back (the hold covers only what follows). Aligning them to her
breathing's grid, or exempting sleep-talk as a stir is, is the user's
call (plan.md).

**Static wakes her on its frames; the hook doesn't:** a use wakes her on
its 1.4 s frame grid and as each key ends, and a client that paints only
when she says something changed would show the 1.2 s of switch-on static
as one frozen frame, which reads as a broken picture. So while a key
shows static she also wakes on its 400 ms frames (`Key::frame_ms`, the
same rule that wakes her on a bob's frames), and all three switch-on
frames are painted (`the_tv_screen_holds_its_programme_through_a_plain_watch`).
Chiyo-chichi's hook still bobs at paint time, sampled on the 1.4 s grid:
waking her every 400 ms through two fifths of a shopping act would make
him bob more visibly, and his bob is talk, not a picture to see whole.

**The hook's length is the lingered body's:** the hook is two fifths of
the shopping act's body (`Span::Upto(2, 5)`), and the body lingers by
mood, so a lazy shopping act (up to 82 s × 1.5 = 123 s) bobs for up to
49 s, against 33 s before watching lingered. **The user left it (step
12c):** "Continuous cyclical movement gets filtered out by the human
optical system almost as quickly as static scenes." Steady periodic
motion is not what draws the eye; a change is (design.md's stillness
rule says so). So the hook isn't capped in ms.

## The film on her TV (2026-10-07)

**Rule:** In line art, with a film loaded that this client holds, her
TV shows a still of it in place of her drawn programme: asked of the
player as she heads to watch, then once a minute (since the last
question, failures counted) while she watches; a good still cuts in at
the next paint; a failure keeps the still; a change of film clears it.
The frame goes through the TV's own private screenshot slot and never
leaves the process. See [design.md](design.md#houseguest).

**Why:** the user's idea ("could it use screenshots from the running mpv
player, if any (changing infrequently)?"): her TV showing what the room
is watching. It is a **held picture, not animation**: step 12a made the
TV hold still because a moving screen in a TV home outshone her, and a
still a minute keeps that. The user chose the mock's column 7 (zoom 1.3,
one luma levels stretch, saturation ×1.3, no sheen) over plain crops
(night and interior scenes went to mud at 27×22 pixels), posterizing,
scanlines, a CRT tint and letterboxing (each worse in the mock), and a
refresh about once a minute during a long watch over the synthesis's
"one still per switch-on, never swapped" (Q2). The saturation is the
column's own ×1.3 (step 12c): the Q1 answer and step 12b wrote ×1.25,
but the user chose the column by eye, and the column applies ×1.3.

**Why its own screenshot slot:** AI commentary deletes its `frame.jpg`
before each request and sends what it reads to the Anthropic API.
Sharing that path would let commentary delete the TV's frame mid-poll,
or attach a frame the TV asked for to an API request. So the slot and
its poll are a module of their own (`screenshot::Slot`), each user with
a private 0700 directory; the TV's frame is decoded, treated and deleted
in the process, `TvPicture` and `UiInput` have no `Debug`, and the logs
say how long a frame took or why it was rejected, never its pixels
(`slots_are_private_and_apart`).

**Why only a held file, its real video showing:** without the file mpv
shows dessplay's "You don't have this file" placeholder, which at 27
pixels is a smudge. So the UI asks only while this client holds the
now-playing file ready (`held_now_playing`). That alone wasn't enough
(step 12b review): `Ready` comes on prefetch, before the player loads the
file, and when a download finishes while the placeholder (a `Load` that
reuses the file's hash) still shows; and the UI asks at once on a change
of film, right inside that gap, so the placeholder or the last frame of
the episode before would have been shown under the new film's name for
a minute. So the gate is at both ends, for the TV and commentary alike:
the session asks the player only when it told it to load the file's
*real* video (`PlayerWiring::may_screenshot`: held, and the
`holds_now_playing` that gates speaking for the group), and the player
actor takes a screenshot only of the file it was last told to load,
once the player's own path echo confirms it shows it (not in the gap
after a `Load`, nor while the user's own dropped-in file is up),
answering at once whether it took it. A question the player wasn't
asked costs no minute ("failures count" is about frames that failed):
it's asked again 5 s on, so an episode's first still comes within
seconds of its video showing, not a minute later.

**Why numbered questions and one frame at a time:** a question given up
after 3 s can still be answered later, and every request clears the
slot's one path: two polls could race on it, one deleting or reading the
other's frame. So each question carries a number and only the answer to
the one out counts, and the slot is claimed for one frame at a time
(`screenshot::Slot::claim`; a request while a poll still runs is "not
asked").

**Why invalidation by file, not clock:** a nine-minute-old frame of the
episode still playing is right; a thirty-second-old frame of last
night's is wrong. Each still carries the file it's of; a change of film
(or the file no longer held) clears both slots, and an answer for a file
since changed is dropped. A failed or black refresh on the same file
keeps the still.

**Why two slots and a latch at paint:** the guest's `Graphics` keeps the
still on show and the one delivered since; at each paint the delivered
one takes over (so it cuts in at the paint its arrival triggers). An
image is keyed by `Look::Film(id, programme)` (a pixel hash, so a paused
film's same frame is one image), and composing it needs the still's
pixels: a key is only made while its still is in a slot, and one whose
still is gone draws its programme (the card the act drew in 12a, which is
why the key carries it).

**Why the guest stays pure:** whether a still arrived, and when, depends
on mpv's timing. So nothing she does reads it: the want
(`Guest::tv_wants_picture`) is output only, the still goes into her
drawing alone (`Guest::set_tv_picture`, never `IdleView`), her card is
drawn on every watch whether or not a film covers it (12a), and there is
no reaction line (Q3: the user is thinking about Osaka commenting on the
episode itself later). `her_film_never_moves_her` checks it: for random
schedules of answers (good, failed or not asked, up to 4 s late, past
the 3 s give-up), stray stills,
clears and changes of film, her trace and every ASCII frame are
byte-identical with and without the stills reaching her, in both modes
(a leak of the latch into her generator fails it at once).

**Why the screenshot goes out async:** measured on this machine with
`--vo=null`: mpv answers a `video` grab in 73–128 ms at 3840×2160 (h264)
and 41–63 ms at 1080p (HEVC), and a command sent behind it waits as
long, since a screenshot holds mpv's command queue. With `"async": true`
the queued command is answered at once and the screenshot's own reply
still comes when the file is written, so every `screenshot-to-file`
(commentary's too) goes out async. A real `--vo=gpu` with hardware
decoding may differ; the session logs each frame's time at trace.

## Where she unpacks a parcel is judged with it there (2026-10-07)

**Rule:** A delivery comes in only where her seats' own test
(`seats_of`) offers an `Unpack` seat at the box, on the room as it will
show with the box in it, and a seat for every use of the piece that
asks room (and to look out of a window, where it first tries that), on
the room as it will show with the piece out of its box: in each, the
pieces that made way where they'd be, and what she made that it leaves
standing (judged as `tend_made` will judge it: one predicate,
`made_stands`). It's asked of this frame's terrain, read once when a
delivery first asks. The wall clock's gift goes through the same door.
See [design.md](design.md#houseguest).

**Why:** a random case of the parcel property
(`every_parcel_on_her_doorstep_gets_unpacked`, seed 0, found in 5c
step 8c, kept ignored since): in line art a sofa came in beside her TV
on The List's floor and was never unpacked. The doorstep asked "can
she stay at its unpack spot?" of the terrain as it stood *before* the
box: but in line art the image she's drawn in takes in every piece it
meets, so once the box stood there her image at that spot took in the
box, then through it the TV, then the letter above the TV, and she
couldn't stay; the visit offered no `Unpack` seat. The same judgement
of a room without the piece in it had been fixed for makeshift pieces
already (`builds` judges where she'd crumple and use one with it in
her image). It was there from the rule's start (78c53d98, 5b: the case
fails there, the first commit with the rule and its test), not a 5c
regression. ASCII is immune (pieces are no part of where she may stay
there); the TV's parcel, every piece sold and her clock all come by
the doorstep, the stage's `give` places a piece out of its box (no
unpacking, stage-only), and later changes of the frame (text she
moves, a resize) can still take the seat away after it's delivered, as
for any seat, which the rule doesn't promise against.

The review of that fix found its sibling: room *to use* the piece was
still asked of the frame's cells alone (`roomy`), not as her seats are
(in either mode, a floor under her; in line art, her image clear of
text). The widened property (out of its box, nothing moved since it
came, a seat for every use that asks room) found a desk at The List's
wall with a letter in the floor at its stool, in ASCII: she could
unpack it, never sit at it. Both are now asked of `seats_of` (`roomy`
still holds too: the piece fits with her clear of the other pieces).
The unpack check now accepts any of her `Unpack` spots, as the visit
does (it had asked only spot 0). The doorstep had also judged on the
visit's terrain, which is last frame's (read after `furnish`), on a
resize a grid of another size; it reads this frame's now. And what she
made that a delivery leaves standing was approximated by "its cover
meets no piece"; it's now `tend_made`'s own test. The property's owned
pieces now range over every kind (hung decor too), and the cases it
found are named in `parcels_she_could_not_unpack`.

The review also turned up a wall clock delivered beside her sofa, in
ASCII, with a seat to unpack it from all along, still boxed ten minutes
on: not this class. Unpacking it was the top of her offers at every
choice (61.7, 65.3, 20.0, 46.7) and she rolled something else four
times, phase 5c's lingering leaving few choices in ten minutes. Her
strongest wish is not a certain one (she rolls among the top few:
brain.rs), so the property's ten-minute deadline was a deadline over a
roll. It now checks what is promised: while it's boxed and nothing has
moved, every choice she makes has unpacking on offer (bound), and
chosen, she finds a way there; once she sets about it, it's out of its
box within two minutes; and she hasn't passed it over 14 choices
running (the case runs on to 50 minutes for that; over 2000 cases the
count fell off by about 0.35 a choice, none past six, so by luck alone
one case in some hundred thousand). The case is named,
`a_clock_she_passed_over`.

## A moved wide glyph rains whole (2026-10-07)

**Rule:** In the goodbye rain (and a focused pane's), a wide glyph she
moved is one brick with the cell after it: while it holds it is drawn
whole, and that cell's own frozen cell (the hole another moved glyph
left there, say) shows nothing of its own; a drop reaching it rains
both as two narrow cells, and they settle together; a change of the
real UI under either half settles it at once. Only a cell painted
before the glyph (under its second half) is that brick's; one of hers
painted after it knocked it out, as in the frame she painted, and the
glyph settles when that cell does. Every cell her figure, pieces,
bubbles, door and rain write goes through one writer (`cells::put`),
which takes a wide glyph's second cell with it (or refuses it), so no
frame of those can hold half of one; the text layer, the parcel flap
and the accordion shake write their cells directly, and are immune for
the reasons below. In ASCII a makeshift piece draws a wide letter as a
narrow stand-in (the rain's scramble of it): one glyph a cell. Every
line she says is plain ASCII, a compile error otherwise (`line!`;
since widened to a few narrow marks:
[below](#her-lines-may-use-a-few-narrow-marks-2026-10-07)). See
[design.md](design.md#houseguest).

**Why:** a random case of the day-long property test
(`her_days_never_touch_what_is_protected`, phase 5c step 12b) found
half a wide glyph in the rain after she slid a line with wide glyphs
left and went off to school: the moved 漢's second half sat on the hole
`w` left, and that hole, frozen as a cell of its own, rained noise
beside the still-frozen 漢 (painted after it, so neither knocked the
other out). Two faults met: `put` was written for narrow glyphs and
never cleared a wide glyph's second cell, and the rain froze that cell
as the hole's, not the glyph's. Both are as old as the text layer
(2026-09-28: the new dissolve tests fail there); 5c step 8c's tuning
only led this seed into that slide before a leaving. The dissolve's own
property test froze only `/` and `V`, so it never reached a wide frozen
glyph; it now freezes wide ones too, and a second property drives the
real text layer (slides and carries over rows of wide and narrow
glyphs, someone typing after the freeze). With more cases the two found two more orderings
in the same class, now settled: a cell beside a live wide glyph judged
"still animating" by a partner settling later in the same frame (so it
blanked a glyph already shown real), and a cell judged on the frame as
painted so far, after its neighbour's write had blanked the wide glyph
it was frozen over. A cell is now judged on the live frame as it was
before the rain wrote anything, unless another of her cells was painted
in that same cell this frame (one frozen over the other: then the later
shows while the earlier shows what it showed then, as before); and
"settling" counts this frame's settling, whatever order they're painted
in.

The review of that fix found three more in the class, each with a
deterministic test that failed first: a glyph and its own second half
were judged each other's "partner" beside a live wide glyph (moving
`漢語` two cells left puts 語 on the real 漢), so the half settling by
its own column's clock popped the moved glyph back without rain (a half
now settles only with its glyph, and the two are never partners); what
shows at a partner's cell was read off the cell of hers on top there,
whose `under` is her own lower cell, not the real UI, so a neighbour
settled early (it's now read off the first cell she painted there); and
any cell of hers on top after a moved wide glyph was taken for its
second half, hiding a later bubble or sprite cell the frame showed (now
paint order decides, as above).

The siblings: the layer's own paint writes a wide glyph only on cells
it validated free (both halves), so it never writes half of one;
the flap swings in on a `│` cell (never a second half) with a narrow
glyph; her bubbles say fixed lines, every one written with `line!`,
whose compile-time check now refuses any but plain ASCII (a lint over
the pools alone would have missed some fifty other lines: greetings,
pitches, constants); her sprite, door and furniture art are narrow; the accordion shake rotates an all-narrow row. The one
other writer of torn text, a makeshift piece's ASCII drawing, could
draw a torn 漢 in one cell of its own; with `put` taking second cells,
one at the piece's last column would have blanked a cell outside it,
so it draws a narrow stand-in instead.

## Her lines may use a few narrow marks (2026-10-07)

**Rule:** A line she says is ASCII plus a short allowlist of narrow
marks (`NARROW`: `… ♪ — – ’ ‘ “ ” ·`), each one cell wide under the
width the renderer uses (unicode-width's `width`, not its CJK variant);
`line!` refuses any other character at compile time. See
[design.md](design.md#houseguest).

**Why:** 12d made every line plain ASCII so that no wide glyph could
leave half of itself in a bubble (drawn a character a cell). That threw
out marks that are as narrow as ASCII in every terminal the users run.
The user: *"Realistically none of us use a CJK locale; wide-character
support is useful mostly for rare subtitles and filenames."* These marks
are East Asian Ambiguous, two cells only in a CJK locale, which the
renderer never assumes (it measures every cell with the non-CJK width).
So the rule is an allowlist checked against that width function, not a
general "narrow" test: a test (`her_narrow_marks_are_one_cell_wide`)
pins each mark at one cell under `width` and checks that the code that
keeps wide glyphs whole (`cells::put`, `cells::width`, the rain's
pairing) never treats one as wide. Rejected: allowing any character
the width function calls narrow. That would let in combining marks,
zero-width and control characters, and anything a future
unicode-width revision re-measures, all unchecked. 12d had swapped no
line's mark for an ASCII stand-in (her `...` lines were always ASCII),
so no line changed.

## Her empty home rains out whenever she doesn't come in (2026-10-07)

**Rule:** Her coming in from her empty home (home from school, a dash,
a stage cue) carries the home in her arriving state until the next
paint. If she comes in, the visit shows her pieces from that frame on.
If she doesn't (the client busy as school ends, the arrival called off
by the gate shutting, or nowhere for her door), it rains out
(`Guest::call_off`, through `rain_home`, the one place an empty home
rains out whole; an errand breaking in rains only her door,
`door_rain`, and a focused pane only what stood in it). At another size than it was drawn at, it goes at once,
as a standing home does on a resize. A stage-cued arrival called off by
her not being idle after all (a key, a chat line) puts her back as she
was, her home standing (`Guest::back_out`), and the cause then takes it
as it would with no arrival under way: a visitor's key rains it out, a
resident's key or a chat line leaves it standing. See
[design.md](design.md#houseguest).

**Why:** phase 5c step 1 made the rule that a rain is cut short only
when she's sent away (switched off, moved out) or her room goes, and
handed off one case it didn't reach: with the client busy, `school_out`
set her state to `Absent`, and her empty home vanished with no rain.
The cause was wider than that branch. Every arrival from her empty home
replaced `State::Away` with `State::Arriving`, dropping the home, and
counted on the next paint's visit to show the pieces again. Any way the
arrival came to nothing dropped the home: `school_out` with the client
busy, the gate shutting between the tick and the paint (`observe`'s
`Arriving → Absent`), or no spot for her door at the paint. The
property `a_live_rain_always_gets_its_frame` was widened with that
scene (an overlay up as school ends, a resident, both drawing modes,
any shell lateness) and a check that a shown empty home never goes
`Absent` in one step. It failed on seed 0: the tick at 12:45 decided
her return, the overlay's paint called it off, and the home was gone.
Carrying the home in `Arriving` makes "called off" mean "rain it out"
in every path, and an errand sent from that state rains her door out of
the carried home as it does from `Away`.

The review (T3) found the first cut too harsh in one case and
under-tested in the rest. A key or a chat line over a stage-cued
arrival rained the home out, though the same key or line over the home
itself leaves it standing; calling off an arrival is no reason to treat
the home worse than its cause would, so `back_out` restores it. The
other paths, each pinned in both drawing modes and each confirmed by
breaking the code it covers: `school_ending_on_a_busy_client_rains_her_empty_home_out`
(the reported branch; in the app only a visitor's key on her errand's
last frames reaches it, so the test sets the idle timer as that key
does), `an_overlay_as_she_dashes_home_rains_her_empty_home_out`,
`an_errand_as_she_comes_in_from_her_empty_home_rains_her_door_out`,
`home_from_school_with_nowhere_to_come_in_her_empty_home_rains_out`
(and at another size, at once), and
`a_cued_arrival_called_off_leaves_her_empty_home_as_the_cause_would`.

The siblings, immune: going out by her door (to school, a dash's end,
the no-home leave, a visitor's errand at school) goes through
`out_by_door`, which either leaves her home standing (`Away`) or sends
the visit through `leave()`, raining. Vacations and weekends never make
school time, so no empty home stands then, and the day's latch holds a
vacation flag through the day (A12). `absent()`'s own school's end has
nothing on screen. An overlay, a busy visitor or a visitor's key over
`Away` already went through `leave()`; a resident's key and a chat line
leave it standing. Switched off, moved out, too small a terminal or a
resize still remove it at once, as the rule says.

One exception, known and left: the stage's cue over a goodbye under way
(`Guest::cue` over `State::Leaving`) replaces it with the arrival and
cuts its rain short. It predates T3 and only the stage reaches it.
Pushing the goodbye's dissolve into her rains (`fades`) would keep the
rain but not the goodbye's image and pieces, which it paints over the
dissolve until the rain starts, so in line art it would show a
different frame; and she would arrive under her own goodbye. No golden
scene covers any of these paths, so all 112 traces are unchanged.

## Her door has its own space at the screen's edge (2026-10-08)

**Rule:** one wall of her strips is her external door's (`Home.door`,
saved in her record), chosen by the panes' geometry alone: walls at the
screen's edge first, then her pieces' strips, then where the space
would be kept, then pane order, right before left; never a wall whose
space meets the chat pane. Six columns against it, her height
and the floor row, are kept: the floor lane of that strip packs beside
them, the hung lane keeps out of them but for the poster and the clock.
The space yields (the strip lays out as with no door) when keeping it
would cost the strip a piece. See
[design.md](design.md#houseguest), *Her door's space*.

**Why:** the user's plan for her door (plan.md, Phase 38, "Next: the
door batch"): a reserved space at the screen's edge, floor against a
wall, never over furniture. **Post-mortem:** her school-morning door
stood at her feet, found by `restful`, the test for where she may stay,
which knows nothing of her pieces' covers: on a school morning she
could go out by a door standing in her sofa or her bed. The door's
place is now computed from her home and the frame, never her feet; this
step keeps its space (the door itself moves there in the next step).

**Kept by narrowing, never by `blocked`:** `project` closets a piece
whose cells meet `blocked`, so a space kept that way would send every
older home's piece by the screen's edge to the closet. Narrowing the
floor lane moves them along instead, in order, their anchors unchanged.

**When it yields (the door batch's step 2, found by its proptest):** the
design first had the space kept whenever what stands packed beside it,
and a window hung into it go to the closet. An older record showed what
that costs: the floor six columns narrower pushed her desk into her bed
and her bed a column along, under her window hung at the far wall beside
her poster, with nowhere left for the window to hang, so loading the
record lost the window (`an_older_record_migrates_without_closeting_anything`,
pinned by `an_older_records_window_keeps_its_place_by_her_door`). The
space is now kept only when keeping it costs the strip nothing (what
stands packs beside it, and every hung piece the doorless layout lays
out is still laid out); else the strip lays out byte for byte as with no
door, its window free to hang in the space (her door stands elsewhere).
So whether it's kept depends on where a hung piece is anchored, not
only on what stands: a repair is still judged on the strip as it would
be laid out, so this costs a search candidates, never a wrong move.

**Review of step 2 (2026-10-08):** the door was first re-chosen after
the frame's layout, so the frame her pieces moved laid them out with no
space, and the next frame hopped them six columns; and a moved piece
could land in the new door's space. `move_off` now judges each target
strip with her door already moved there, and the layout follows it. The
tie-break "where what stands packs beside the space" now asks what
`kept` asks (no hung piece lost either), so the chooser never prefers a
wall whose space would yield at once. The design first said the wall
is never chosen again for "a pane hidden a while": that only holds
while no other strip can take her pieces. A hidden pane fails the
layout like a pane that's gone, so her pieces move (as before this
batch) and her door with them, and showing the pane again keeps both
where they went. Left as built; whether a hidden pane should hold her
pieces (and door) in place is an open question for the user.

**Rejected:** a wall derived each frame (the first delivery would move
it, and her pieces with it, and each pane drag would too); moving the
room off its strip to make way (a whole room moves for six columns);
choosing the wall inside `project` (every test of the layout would get
a door; `Home::frame` settles it before the layout instead, and
`project` only reads it). A saved wall the chat pane comes to meet is
not chosen again: it's refused frame by frame, so dragging the pane back
restores it.

**Where her door stands (the door batch's step 3, 2026-10-08):** her
closed door (while she's out) and the door she comes home and dashes in
through are placed by one function, `door::door_place`, from her home,
the frame's panes and lines, and her pieces' covers as laid out (and,
on a visit, what she made and the place of the piece in her pocket):
in its space when kept and free; else at the nearest floor spot where
her box meets no piece, clear of the chat and protected cells; else
nowhere. Nothing records her feet any more (`DoorAt` and `Out`'s spot
are gone; the last spot placed lives on her empty home, derived by each
frame), and only `door_place` makes a `DoorSpot` (its fields are
private). The acts that open her door (`back_through_door`, `dash_in`,
`dash_through`) still take a bare spot and facing until step 4a
(`Through::Home(DoorSpot)` / `Through::Space`): coming home with no door
anywhere opens that act at a spot in space. With no door anywhere she
comes in somewhere calm outside the chat if there's such a spot, else
anywhere clear of it and her pieces. Her door reads text she moved
during a visit as text, never as a protected pane, and an empty chat
rect (a collapsed slot) is no chat (`room::in_chat`). **Post-mortem:** the old door stood where she went out, and
`door_fits` judged it by `restful`, the test of where she may stay,
which knows nothing of her pieces: out of her bed, sofa or desk at
08:15 she went out by a door standing in it, and on the chat pane's
floor by one in the chat (`her_school_mornings_never_put_her_door_on_a_piece`
and the step's `out_of_her_*` tests were red on it). The fallback is
blind to text and sticky (`prev`): a door that moved as text scrolled
would draw the eye. *It stands over text* (the user's answer): text in
its space neither moves nor hides it; only a protected pane does, and
then only while that pane is in use (the drawn door reads the frame as
drawn; arrivals and the visit's door read it as she keeps to it, so she
never comes out inside a focused pane). The image of the door takes in
no piece: it meets none by construction. What she makes is kept out of
the space too (`door::Keep`: `builds` refuses it, a piece she set out
to make there after the space came is refused as it's made, and a made
piece the space comes to meet falls apart), the chat half following in
step 5.
*Rejected:* hiding the door while text is in its space (the user chose
the door over the text: it's how you know she's out); keeping the old
spot while it still fits (that's the bug).
*A face-on door never straddles a pane's border* (the user's answer,
2026-10-08, step 4a): D4's strict fallback read no line above the floor
row, so a fallback door (and her coming home out of it) could stand
across a pane's `│`, half in each pane. `on_floor` now refuses a plain
wall (`│`/`┃`) on any of her box's rows above the floor; the floor
row's own stroke is floor. *Rejected:* letting it straddle (it read as
the door being in neither room).

**Older builds:** the record keeps `door` after what she has seen and
before her clock sent; an older build drops it when it saves the record
again and lays the door strip's pieces against the raw wall, those
anchored from the door's wall up to six columns nearer it, until a newer
build chooses the wall again. Shares (`at`) are always of the raw strip,
so nothing is lost; the version stays 1.

**Her repair search lays out only the strips a move changes (the door
batch's step 3p, 2026-10-08):** each strip is laid out alone and
recorded with what it was laid out from (her pieces on it, by index, and
her door's wall if it's there: all its layout reads). A move's layout
reuses a recorded strip only when those are identical, and lays out the
rest again. *Why:* the search weighs ~1170 moves, and laying out her
whole home for each (her door's strip twice, to judge its space) made a
search 0.60-0.76 ms, past its 1 ms bound under a deep pass's load; now
0.43-0.48 ms. Her door's space is kept without laying the strip out a
second time when, laid out beside it, the strip leaves none of her
pieces out (`lay_strip` counts them, whatever leaves them out): without
the space it could show no more. *Rejected:* the caller naming the
strips a move touches (the first cut): a wrong list, or a new input to a
strip's layout (the chat, made pieces), would reuse a stale strip with
no error; recording the inputs makes that unrepresentable, and comparing
a few pieces costs far less than laying a strip out. Laying out again
after a resize (`move_off`) and per place of a stage gift (`spot`) still
lays out the whole home: they're rare, not per move.

## Houseguest chooses by needs among the top few (2026-09-28)

**Rule:** Her next act is a weighted-random pick among the four
best-scoring offers, scored by base × (floor + need²) × cooldown. See
[design.md](design.md#houseguest).

**Why:** The phase 1 roll table gave every visit the same texture: she
dozed as often in minute one as in minute thirty, and "busier early" had
to be a special case. Needs give a visit a shape without scripting it.
Argmax was rejected as robotic (the pressing need always wins, the same
act again and again); flat random as a slot machine (needs would barely
show). The squared need keeps half-full needs quiet — with a linear fit
she dozed a quarter of the time straight after arriving — and the floor
keeps every offer possible, so no need can starve the rest (the
Tamagotchi lesson: needs flavour, never punish). A doze eases sleepiness
only a little: when it reset it, sleepiness oscillated at the same level
all visit and the "sleepier later" shape vanished. Only needs with both
a driver and an answer today exist; `hungry` and `social` wait for food
words and cameos. *(2026-10-02: needs are now eased by how much she does of what she
chose, not the moment she chooses it; and comfort, fun and daydreams
joined them, so every want but standing and sneezing answers a need;
see [her mind is a table and a heading](#her-mind-is-a-table-and-a-heading-2026-10-02).)*
