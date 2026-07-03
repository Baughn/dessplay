# DessPlay Design Document

Last updated: 2026-07-03

A synchronized video player for watch parties. Terminal-first, built for
reliability over flaky connections. Server-coordinated, including relayed
file transfer between peers.

## Table of Contents

1. [User Experience](#user-experience)
2. [Client Roles](#client-roles)
3. [Presence](#presence)
4. [The List (Series Tracker)](#the-list-series-tracker)
5. [TUI Layout](#tui-layout)
6. [Network Protocol](#network-protocol)
7. [File Management](#file-management)
8. [Player Integration](#player-integration)
9. [Data Storage](#data-storage)
10. [Security / Threat Model](#security--threat-model)
11. [Key Definitions](#key-definitions)

For internal architecture (actors, message flow), see [architecture.md](architecture.md).

---

## User Experience

This section describes the full workflow from a user's perspective.

### First Launch

1. **Launch DessPlay** from the terminal: `dessplay`
2. **Settings screen** appears (automatically on first run; reopen any time
   later with `F3` or the `/settings` chat command):
   - Enter your username
   - Choose your player (mpv or vlc; terminal version only)
   - Add media root directories (where your anime/shows live; terminal version only)
3. **Main screen** appears with chat pane, users list, playlist and video library

### Settings Screen

The settings screen includes several required settings:
- Username (Defaults to $USER on Linux/OSX, equivalent on Windows)
- Server (Defaults to dessplay.brage.info)
- Ready on startup (Toggle, defaults to off). When off, the user starts as
  Paused on connection. When on, the user starts as Ready.
- Media roots (Selected by file browser; at least one must be selected).
  The topmost media directory is listed as "download target" (blue text on the right).
  Media roots can be reordered with `J`/`K` (or `j`/`k`) -- bare letters
  rather than Ctrl-J/Ctrl-K, which collide with control codes (Ctrl-J ==
  LF) in terminals lacking the enhanced keyboard protocol, consistent with
  the playlist pane's reorder keys.

Optional settings (sensible defaults, editable later):
- Cache retention (duration; `0` = delete watched downloads at end of session,
  `infinite` = keep everything; see [Download Cache](#download-cache-and-retention))
- Upload limit (bytes/sec cap for serving files to peers; default unlimited)
- Subtitle mode (off / intermixed / separate pane; default off; also
  cycled live with `F2`). See [Subtitle Display](#subtitle-display).
- Auto-download (toggle, default on). When off the client never fetches
  file contents from peers -- neither the prefetch window nor the missing
  now-playing file -- so it relies entirely on its own media roots. See
  [Pre-fetching](#download-cache-and-retention).
- IRC bridge (toggle, default **on**) plus IRC server (default
  `irc.rizon.net`), IRC TLS (toggle, default on -- selects port 6697 vs
  6667), and IRC channel (default `#dess`). See [IRC Bridge](#irc-bridge).
  A dim hint reminds the user the channel is **public**: chat leaves the
  encrypted group.

Seeder-specific configuration (role, retention) is provided via
command-line flags / environment only -- seeders are headless, never show
the settings screen, and persist no settings (they are run as systemd
services from a NixOS config). See [Client Roles](#client-roles).

### Connecting to Friends

1. DessPlay connects to the rendezvous server (QUIC, TOFU certificate trust)
2. The rendezvous server establishes a shared clock via an NTP-style protocol
3. The server provides the current state snapshot and any pending operations
4. Connected users appear in the **Users pane** (top-right area)
5. Connection happens automatically on launch; no manual action needed

All state synchronization goes through the server. Clients do not directly
sync state with each other. See [network-design.md](network-design.md).

### Adding Files to the Playlist

**From the Series pane:**
1. Press `Tab` to focus the **Series** pane (top-right)
2. The pane has three modes, cycled with `m`:
   - **Recent Series** (default): only franchises the user has *watched*, most
     recently watched first (then title). Unwatched series are hidden. Press
     `/` to filter by title substring (case-insensitive); the filter *removes*
     the watched-only restriction, so any series can be found. `Esc` clears the
     filter. (Filtering is gated behind `/` so the bare `m` / `s` keys stay
     live — and reliable: Ctrl-modified letters collide with control codes,
     e.g. Ctrl-M == Enter, in terminals lacking the enhanced keyboard
     protocol.)
   - **All Series**: every franchise, sorted by title or year (toggle with
     `s`). `/` filters the same way.
   - **The List**: see [The List](#the-list-series-tracker).
3. Related anime are grouped into **franchises** using AniDB's relations graph
   (sequel, prequel, side story, etc.). Each franchise shows as one entry. The
   browser spans the group's **collective library** -- every file any client
   has indexed (see [Media Library Scanning](#media-library-scanning)), not
   just files already in the playlist.
4. Press `Enter` on a franchise:
   - **Single-season franchise**: opens the file browser in the series directory,
     cursor on the next unwatched episode
   - **Multi-season franchise**: opens the **Episode Browser** modal showing
     seasons (franchise members). Select a season to see its episodes.
5. In the Episode Browser, press `Enter` on an episode to add it to the
   playlist. If you have the file locally it resolves Ready; if you don't, it
   is added anyway (using the file catalog's identity) and downloads like any
   other missing file. Press `Esc`/`Backspace` to go back.
   - Episodes are grouped by AniDB episode identity: most episodes have
     exactly one known file and render as a single line ("Episode 03
     `[Judas] Frieren - 03.mkv` Baughn Nero Kim"); when several files
     claim the same episode number they expand into a lightweight tree --
     a display-only header line plus one selectable child per copy, each
     with its own holders. Holders are the users currently advertising
     that file `Ready`, right-aligned and dim, so "pick the file you
     already have" is a glance away. A file with no parseable episode
     number never merges with another just because it's adjacent --
     there's no evidence they're the same episode.
   - Episodes watched personally (85% history) or by the group (the
     watched flag) render muted, matching the playlist pane's convention.
     A `<` marker sits on the first not-fully-watched row, and the
     browser opens (and a season, once selected) with the cursor already
     there.
   - `w` cycles the selected file's group watched flag directly (a
     `MarkWatched` request to the server, mirroring `EofReached`'s
     watched-flag write): handy for marking an episode watched without
     playing it to EOF, or undoing an accidental advance. Setting it to
     watched also runs the same List `next_ep` auto-advance the EOF path
     gets. No-op on a header row (no single file to act on).
6. Sort mode for All Series is persisted across sessions.

**From scratch:**
1. Press `Tab` to focus the **Playlist** pane (bottom-right)
2. Press `a` to add a file
3. The browser opens **on the selected entry's local file** when it has
   one (pressing `a` on the just-watched episode is the common way to
   queue the next one, which then sits a keypress away); otherwise it
   opens at the media roots
4. Navigate your media root directories — directories are colorized,
   and files you (or the group) have watched are greyed out, matching
   the playlist's muting
5. Or **type to search**: any typed text filters the *whole library
   index* recursively (case-insensitive substring over root-relative
   paths, so deep hierarchies don't hide anything). Matching directories
   list first — e.g. `haibane` finds `Anime/Purgatory/Haibane Renmei` —
   then matching files; selecting a directory clears the search and
   browses it. `Esc` clears the search; Backspace edits it. The search
   spans the client's **library index** (see
   [Media Library Scanning](#media-library-scanning)), so files not yet
   indexed are found by navigation, not search. The manual-map browser
   searches the same way; the media-root *directory picker* does not
   (its `s` key stays live, and the whole filesystem has no index).
6. Select file to add. (Enter)

**Reordering:**
1. Focus the **Playlist** pane
2. Use `J` / `K` (lowercase `j` / `k` work too) to move the selected item
   down/up; the cursor follows the moved entry, so repeated presses keep
   carrying the same episode

### File Matching

When someone adds a file, everyone needs to find their local copy:

1. DessPlay searches your media roots for files with the **same filename**
2. If found: file appears normally in your playlist
3. If not found: filename appears **red** in your playlist
   - **Known series** (the file's AniDB series ID matches something in your
     watch history; before metadata arrives, falls back to "you've watched a
     file whose name matches the same series-name parse"): the file is marked
     **Missing** — this blocks playback, because you probably should have
     this file
   - **Unknown series** (no watch history for that series): you are set to
     **Not Watching** — a generated placeholder PNG is loaded into your player
     showing the current state. *Implementation note (Phase 9A):* the
     automatic Not-Watching set requires an AniDB **series id** to key the
     synced preference on; a missing file whose series has no id (AniDB
     didn't know it, only a filename-derived series name exists) keeps
     blocking, and the manual not-watching action (4b) is the escape
     hatch. The "known series" detection itself uses the series id when
     present and the series name otherwise.
4a. You can manually map to a different file:
   - Select the red entry, press `M` to open browser
   - Browser opens to the directory most recently used for files from that
     series (the main loop supplies it from `series_map_dirs` when the
     browser is requested; unknown series open at the media roots)
   - Files are sorted by edit distance to the target filename
4b. You can manually set yourself to "not watching" on a file that's Missing
   (e.g. a known series but you don't have this episode yet). This clears the
   "missing from known series" block
4c. By default: The file is retrieved from peers using a bittorrent-like protocol.
    Downloaded files live in the **download cache** and are evicted according to
    the retention policy; they are never automatically placed in a media root.
    An explicit **archive** action moves a cached file into
    [Series name]/[Season #]/[Original filename] in the download root, aka. the
    topmost media root. See [Download Cache](#download-cache-and-retention).

### User States

Each user has a state describing their readiness. The default value for this
can be set on the settings screen.

This state is **derived** from two independent sources:

1. **Per-series watch preference** (`Map<(UserId, AniDbSeriesId), LwwCell<SeriesPreference>>`,
   `SeriesPreference { state: SeriesWatchState, set_by: Option<UserId> }`):
   a user's commitment to a specific AniDB series, with **three** `state` values:

   - **Watching** (committed): "I am definitely watching this series." The
     group waits for this user even when they are **absent** -- Lost,
     Departed, or quit. This is the only state that blocks across absence
     (see [Playback Rules](#playback-rules)); unblocking a committed-absent
     user takes a deliberate per-file [acknowledge](#playback-rules).
   - **Maybe** (the default): opportunistic / undecided -- "I'll watch if
     I'm here, but don't hold up the night for me." A **present** Maybe user
     gates playback exactly like a committed one (the group waits for their
     file); an **absent** Maybe user does not block. This is the value for
     every series a user has never explicitly ruled on.
   - **NotWatching** (definite no): the user skips this series and **never**
     gates playback on it, present or absent, and it is not auto-downloaded.

   When the currently playing file belongs to a series, that user's
   commitment to it is one input to their derived state. A user with no
   stored preference for the series is treated as **Maybe** -- the schema
   stores `Watching`/`NotWatching`/`Maybe` explicitly, and an *absent* map
   entry also means Maybe (see [sync-state.md](sync-state.md#series-preference)).

   **Becoming committed** is a deliberate act -- being listed in a
   [List](#the-list-series-tracker) entry's `watchers` set, or a per-series
   action (`/watch`, or the watch-cycle key in the playlist/series pane).
   `Ctrl-R` / "mark ready" does **not** commit: it clears a pause or an
   auto-`NotWatching` back to **Maybe**, never to Watching, so the
   block-across-absence commitment is always opt-in.

2. **Manual override** (`LwwCell<Option<ManualState>>`): The user can manually
   pause (stepping away), which overrides the series-based state. The override
   is cleared when the user explicitly resumes. `ManualState` is
   `Paused | Away { set_by: UserId }`.

**Away**: any user can mark *another* user as Away (`/afk <name>` or `/away
<name>` in chat, or `a` on a user in the Users pane), and a user can mark
*themselves* away (`/away` with no name) -- for when someone walks off without
quitting and would otherwise block playback forever. Away behaves like Not
Watching for playback gating, and is displayed with attribution ("away, set by
Baughn"). It is cleared by a deliberate "I'm here" action from the marked
user's client -- **attempting to unpause the player, or pressing Enter to send
a chat message** -- back to normal. Merely *typing* a chat line (without
sending it) does not clear it, so you can compose a message while still marked
away. With five trusted friends, no permission system is needed.

**Marking others not-watching**: any user can set *another* user's series
preference to NotWatching -- `n` on a user in the Users pane (the now-playing
series), or `/skip <name>` in chat -- the "Kim tool": rule on someone's
commitment to a show without waiting for them to show up, or acknowledging
them file-by-file. Unlike [Acknowledge](#playback-rules) (a per-file
one-shot, re-needed every episode) this is a durable preference change, so
playback stays unblocked for the whole series until the subject's own later
write overrides it (plain LWW -- no special-casing, unlike Away's
clear-by-the-marked-user's-activity rule). The write is attributed to the
setter (`set_by: Some(actor)`), and the narrator names them ("Baughn set Kim
to not-watching Frieren (by Baughn)"); `a`/`n` on the Users pane also work on
a [known-but-offline](#presence) user who hasn't connected yet today, not just
a currently-listed one.

Derived states (manual override wins; otherwise the now-playing series'
commitment decides):
- **Ready**: No manual override, and the current series is **Watching**
  (committed)
- **Maybe**: No manual override, and the current series is **Maybe** (the
  default). Gates only while the user is present
- **Paused**: Manual override is set to Paused
- **Away**: Manual override is set to Away (does not block playback)
- **Not Watching**: Current file's series is marked NotWatching (no manual override)

### File State

Each user has a flag describing their *ability* to play the current file.
It can have one of three values:

- **Ready**: The hash matches, the file is loaded, and it can be unpaused as desired.
- **Missing**: The file doesn't exist, or the hash is mismatched, and none of the
  step 4 options from file matching have been performed.
- **Downloading**: The user's client is actively retrieving the file from other
  clients. Unpausing is conditional: their download speed must be higher than the
  file's computed bitrate, *and* at least 20% of the file must be downloaded.
  *Implementation status (2026-06-28):* only the **20%** half is enforced today
  (`derive::file_block_reason` blocks below 20%). The download-speed-vs-bitrate
  half is **deferred** — `FileAvailability::Downloading { progress_bps }` carries
  only progress, no measured throughput, so the speed clause cannot be evaluated
  from synced state; honoring it would require a synced eligibility signal. See
  [Future Plans](#future-plans). Impact is a self-only edge (a user who unpauses
  at exactly 20% with throughput below the bitrate may stall their own playback;
  it never gates the group).

### Ready States (UI Display)

Each user has a ready state shown by state & color in the Users pane.
Their ready state is decided by a combination of the above; this only exists in the UI.

| State | Color | Meaning |
|-------|-------|---------|
| Ready | Green | (Ready, or a present Maybe) & Ready |
| Paused | Yellow | Paused & Any (blocks like red; yellow says "a friend paused", not "something is broken") |
| Away | Gray | Away & Any (shows who set it) |
| Not watching | Gray | Not watching & Any |
| Committed, away | Red | Watching (committed) & **absent** (Lost/Departed) -- blocks; acknowledge to play past |
| Downloading | Green | Ready & Downloading [complete enough to play] |
| Downloading | Blue | Ready & Downloading [still fetching] |
| Downloading | Red | Paused / Away / Not watching & Downloading |

A present **Maybe** user displays exactly like Ready (the per-series
distinction lives in the playlist's right-aligned watch tag, not the
Users-pane colour) -- both gate on their file state while present.

An in-progress download is **always** shown: a peer actively downloading
the now-playing file reads as Downloading even if their derived state is
Paused, Away, or Not Watching -- it must never be shadowed by those
labels. The colour carries the rest of the story: green once it can play
and they are Ready, blue while a Ready peer is still fetching, and red
otherwise (the download is visible; the red says they still won't be
watching right now).

Departed users (see [Presence](#presence)) are shown on the dim, italic
known-offline line -- **except** a committed (Watching) absent user, who
keeps gating the now-playing file and is surfaced as a "committed, away"
blocker until they return or the group [acknowledges](#playback-rules) past
them. Seeders are not listed as users; they appear on a separate dim
"seeders:" line.

The video player carries a persistent **"Waiting for …" OSD overlay**
whenever someone blocks playback of the now-playing file: every blocker
with a short reason ("Waiting for Kim (downloading 34%), Nero (paused)"),
derived from the same gating derivation as the Users pane so the two can
never disagree. It is shown to **everyone** — including the blockers
themselves — and cleared the moment nobody blocks. Implemented as an mpv
`osd-overlay` (top-right), independent of the chat OSD, so chat traffic
never hides it and it survives player relaunches.

**How states change:**

- **On join**: User State starts as Ready or Paused (depending on "Ready on startup"
  setting); File State depends on whether the file was found locally
- **Missing file (unknown series)**: User State -> Not Watching; File State -> Missing;
  placeholder text loaded into player. *Suppressed when the file is
  obtainable* -- if a present peer (typically the seeder) advertises the
  file Ready, it downloads instead of writing a sticky Not Watching, and
  the placeholder shows while it arrives. (A residual race -- the source's
  Ready not yet synced when the decision fires -- can still set Not
  Watching once; the Downloading display masks it and Ctrl-R clears it.)
- **Missing file (known series)**: File State -> Missing (blocks playback)
- **Missing file (downloading enabled)**: File State -> Downloading; placeholder is
  updated with download progress
- **Manual pause** (in player): Manual override -> Paused
- **Attempt unpause** (in player): Manual override -> None; unpauses if all users permit
- **Mark ready / unready** (`Ctrl-R`, global): toggles your own readiness
  without touching the Users pane. Marking ready clears your manual
  override, latches playback intent Playing, **and** flips the
  now-playing series back to **Maybe** if it was marked Not Watching --
  the path to undo an auto- (or self-) Not Watching. (It does **not**
  commit you to Watching; that is a deliberate act -- see below.) Marking
  unready pauses (manual override Paused, intent Paused).
- **Marked Away** (by another user, or yourself via `/away`): Manual override
  -> Away; cleared when the marked user's client unpauses the player or sends a
  chat message (not by merely typing)
- **Set watch state** on a series (`/watch` / `/maybe` / `/skip`, or the
  watch-cycle key in the playlist/series pane): series preference updated to
  Watching / Maybe / NotWatching. Setting NotWatching also clears a "missing
  from known series" block when applicable.
- **Acknowledge a committed-absent blocker** (`/ack`, or from the blocker
  line): records a **per-file** one-shot that lets the group play past a
  committed (Watching) user who is absent. It is scoped to the current
  now-playing file -- advancing to the next episode re-raises the block, so
  it is re-acknowledged consciously each file (see [Playback Rules](#playback-rules)).

### Playback Rules

1. The video plays iff the shared **playback intent** is Playing **and**
   no interactive user *blocks*. Whether a user blocks depends on their
   commitment to the now-playing series and their presence (seeders never
   gate; a user with no peer entry is ignored):

   - **NotWatching**: never blocks (any presence).
   - **Away**: never blocks (any presence) -- this is also what an
     [acknowledge](#playback-rules) writes.
   - **Maybe** (the default): blocks only while **present** and not
     ready-to-play (a Missing/insufficiently-downloaded file, or a manual
     Paused). An **absent** (Lost/Departed) Maybe user does **not** block --
     we don't hold up the night for someone who isn't here and only *maybe*
     wanted this.
   - **Watching** (committed): blocks whenever they are not ready-to-play,
     **including while absent** (Lost or Departed) -- "wait for me even if
     I've been gone a week." A committed-absent user blocks until they
     return ready, or until the group [acknowledges](#playback-rules) past
     them for the current file.

   The intent is a synced register (`LwwCell<PlaybackIntent>`,
   `Playing | Paused`) written by users (play/pause actions) and the server
   (forced to Paused on Lost, on graceful quit during playback, and on
   EOF-advance) -- it is the latch that keeps playback paused after a
   blocker leaves, instead of silently auto-resuming. (The server forces
   this Paused on **any** Lost, committed or not; gating then decides
   whether pressing play resumes -- for an absent Maybe user it does, for a
   committed one it does not until acknowledged.) The **timeout-ladder**
   Lost->Departed promotion does *not* re-force Paused: the peer was already
   paused at its Lost transition 30s earlier, and re-pausing would clobber a
   resume the present users legitimately made during the Lost window (an
   absent Maybe user is non-blocking). Only the graceful-quit *immediate*
   departure (which skips Lost) force-pauses.

   **Acknowledging a committed-absent blocker** is a deliberate per-file
   one-shot: it records `(now-playing file, absent user)` in a synced set
   (`acknowledged_absent`), which suppresses that user's committed-absent
   block *for that file only*. Advancing now-playing (EOF or manual select)
   leaves the old entry behind, so the block re-raises on the next episode
   and is re-acknowledged consciously. The per-file scoping is why this is a
   dedicated set rather than a reuse of the per-user **Away** override (an
   Away would persist across episodes until the user returned). The set is
   grow-only and cleared at compaction, like other ephemeral session state
   (see [sync-state.md](sync-state.md#acknowledged-absent)).
2. If you press play in your player but someone is Paused or has a Missing file:
   - Your player is immediately re-paused
   - You are marked Ready, and intent is set to Playing (you tried!) --
     playback starts the moment the last blocker clears
3. When someone pauses, everyone pauses: pausing sets both your manual
   override (so others see *who* is blocking resume) and intent to Paused.
   Pressing play clears your own override and sets intent to Playing.
4. When someone seeks, everyone seeks (via seek authority; see [sync-state.md](sync-state.md))
5. **Drift correction** aligns each client to a **position reference** using
   three bands (thresholds configurable in one place; defaults below):
   - **< 100ms**: ignore
   - **100ms - 3s**: slew -- adjust playback speed by up to ±2% (mpv `speed`
     property, pitch-corrected, invisible to the viewer) until converged
   - **> 3s**: hard seek

   The position reference is the **seek authority's** position when a *user*
   holds authority -- **but only when that user is a valid same-file source**
   (present and advertising `FileAvailability::Ready` for the now-playing
   file). A user can hold seek authority without being on the real video --
   e.g. a not-watching client whose player shows a placeholder, which still
   reports a position. Following such an authority blindly froze the whole
   group on its bogus position (everyone hard-seeking back every couple of
   seconds). So an invalid user authority is treated exactly like Server
   authority below; it is never followed. (Symmetrically, a client that does
   not hold the real now-playing video never *takes* seek authority or
   publishes a position from its placeholder in the first place -- see the
   `holds_now_playing` gate in [Player Integration](#player-integration).)

   The authority is the **Server** for most of an episode -- it is set to
   `Server` on every EOF-advance and manual now-playing change, and only a
   manual seek hands it back to a user -- and the Server has *no position*.
   In that case (and when a user authority is not a valid source) each client
   falls back to following the **furthest-ahead present peer that has the
   now-playing file loaded** (advertises `FileAvailability::Ready` for it):
   the "leader". Following the leader makes laggards catch up *forward* (no
   group rewind); the leader, and anyone tied with or ahead of it, follows no
   one, so the group converges on the front.

   Eligibility -- for both the leader election and validating a user
   authority -- is restricted to peers whose position is **for this file**.
   Two gates, both required: the peer advertises `FileAvailability::Ready`
   for now-playing, **and** their `PlaybackPosition` carries a `file` tag
   equal to now-playing. The file tag is the load-bearing one: `Ready` alone
   is *not* sufficient because it is set on **prefetch** (a peer advertises
   Ready for next week's episode long before it plays it), so right after a
   now-playing transition a peer can be Ready for the new file while its
   position register still holds the *previous* file's sample. Following that
   stale value latched the whole group forward onto it (leader election only
   moves forward, so nobody pulled back to 0:00) -- the new file came up at
   the previous file's position instead of T=0, on both EOF-advance and
   manual selection. The tag is a clock-free identity check; it deliberately
   excludes absent users, users on a different file, and users watching a
   placeholder (file missing / still downloading / not watching) -- their
   position is stale or for another file and must never elect a bogus leader
   or be followed as authority. *Without the leader fallback the player ran
   open-loop under Server authority: any initial offset (e.g. a player that
   started late, or a brief decode stall) sat uncorrected for the whole
   episode, since no `SyncTo` was ever issued.*
6. Seeks are debounced (1500ms) -- only broadcast after the user stops scrubbing
7. **EOF** advances the synced now-playing pointer to the next playlist entry.
   The server initiates this (it is the authoritative entity for "file ended"):
   clients whose player reaches end-of-file send an `EofReached { file }` report
   to the server; when the server receives the first report matching the current
   now-playing file from a present, watching user -- Ready (committed) or
   Maybe, but not a seeder and not one whose derived state is Not Watching,
   Away, or (manually) Paused -- it marks the file watched,
   advances now-playing, sets playback intent to Paused (the next episode
   loads paused; anyone presses play when ready), and takes seek authority.
   Later duplicate reports no
   longer match now-playing and are ignored, making the transition idempotent.
   Files are **not** removed from the playlist on EOF -- they remain visible
   in muted colors as play history. Users can select any entry with Enter to
   set it as now-playing. A manual selection of a **different** file mirrors
   the playback-state half of the EOF transition: it loads the new file
   **paused at the start** -- the client writes playback intent Paused
   alongside the now-playing change, and the server resets seek authority to
   Server on the now-playing op -- so the group presses play when ready, just
   like an EOF advance. It deliberately does **not** mark the abandoned file
   watched or advance The List (selecting a different file abandons the
   current one rather than finishing it). Re-selecting the entry that is
   already now-playing is not a transition and does not pause.

### Before Playback Starts

Before unpausing is allowed, DessPlay verifies file contents match:

1. Compute ed2k hash of the local file
2. Compare hashes across all Ready users
3. If mismatch: unpause is blocked, File State is set to Missing

This prevents sync issues from different encodes/versions.

### Chat

- Type in the chat input (always visible at bottom of chat pane)
- Press Enter to send
- Messages appear in the chat pane AND as OSD in the video player — a
  rolling overlay (top-left) holding the recent messages: each line stays
  a minimum of 8 seconds and expires individually, so a burst never
  erases an unread line (mpv `osd-overlay`, not the single-slot timed
  `show-text`, which each new message would clobber). Your **own**
  messages are not echoed to your OSD
- **Username tab-completion**: pressing `Tab` completes the word at the end
  of the input when it is a non-empty, case-insensitive prefix of an online
  username (present or lost interactive peers; seeders and departed users
  excluded). When the buffer is *nothing but* that prefix the completion
  appends `": "` (the IRC "Baughn: " address form); mid-sentence it just
  fills in the name. If several names match, repeated `Tab` (without an
  intervening edit) cycles through them. When the trailing word matches no
  username, `Tab` keeps its normal job of cycling panes -- so completion is
  invisible until it's useful.
- **Mention highlighting**: in the chat log, any word matching an online
  username is drawn in that user's [palette color](#subtitle-display) + bold
  (trailing punctuation like `:` or `,` is matched-through but stays plain).
  Mentions of *your own* username are additionally reversed, so a ping stands
  out at a glance.
- System messages (joins, disconnects, state changes) appear in chat --
  see [System Messages](#system-messages)
- Text commands start with `/`. Typing `/` shows a grey, filtered list of
  the available commands at the bottom of the chat pane (discoverability);
  it narrows as more of the command is typed and disappears once the input
  no longer matches one. An unknown command (or one that can't run, e.g.
  `/skip` with no series info yet) posts a local-only system line. The
  commands:
  - `/quit` (aliases `/exit`, `/q`; also Ctrl-C) -- quit DessPlay
  - `/ready` -- mark yourself ready (same as the "become ready" half of
    `Ctrl-R`: clears your manual override, latches Playing, and flips the
    now-playing series back to Maybe if it was Not Watching -- it does
    **not** commit you to Watching)
  - `/pause` -- mark yourself paused (manual override Paused, intent Paused)
  - `/away [name]` -- mark yourself (or, with a name, another user) as Away
    (alias `/afk <name>`; see User States). Marking yourself Away holds until
    you unpause the player or send another chat message.
  - `/watch` -- **commit** to the now-playing file's series (sets your
    per-series preference to Watching, so the group waits for you even when
    you're absent; needs an AniDB series id)
  - `/maybe` -- set the now-playing file's series to Maybe, the opportunistic
    default (needs an AniDB series id)
  - `/skip` -- stop watching the now-playing file's series (sets your
    per-series preference to NotWatching; needs an AniDB series id)
  - `/ack` -- acknowledge the current committed-absent blocker(s): a per-file
    one-shot that lets the group play past a committed (Watching) user who is
    Lost/Departed, and latches intent Playing. Re-needed on the next episode.
  - `/summon` -- ping everyone [known but offline](#presence) on IRC in one
    PRIVMSG, with the mandatory Dess-girl link. Deciding "IRC bridge
    disabled" or "everyone's here" needs no round trip (both are already
    known client-side); matching each absent username to a live channel
    nick (by edit-distance similarity, e.g. `Nero` -> `Nero200`, excluding
    `*Dess` bridge echoes) happens in the IRC actor, which tracks channel
    membership from NAMES/JOIN/PART/QUIT/NICK. A local system line reports
    who was pinged (by the nick actually addressed) and who had no
    plausible nick.
  - `/me <action>` -- send an IRC-style action ("* Baughn waves"). Unlike
    the other commands this is a real, **synced** chat message (it reaches
    everyone, persists, and shows on the player OSD as "* Baughn waves");
    sending one also clears your own Away. The action is carried inline in
    the message text using the CTCP `ACTION` convention
    (`"\x01ACTION waves\x01"`), so no separate message type or schema change
    is needed -- only the display sites decode it. In the chat log the
    action phrase renders **grey** (terminals have no italics, so colour
    marks the emote); the sender keeps its palette colour and mentions
    still highlight through it.
  - `/settings` -- open the settings screen (also `F3`)

### IRC Bridge

DessPlay logs are unavailable when the program isn't running, so the
chat is gone the moment you close the app. The IRC bridge fixes that:
each interactive client (never a seeder -- they have no chat) optionally
mirrors **its own** chat into a shared IRC channel that others can keep
open or log, and surfaces messages from plain-IRC users back into the
chat pane. It is **on by default**; defaults are `irc.rizon.net`, TLS
(port 6697), channel `#dess`.

- **Identity.** The client connects as `[Username]Dess` (e.g.
  `BaughnDess`). The username is sanitized to a legal IRC nick (illegal
  characters dropped, a letter forced to lead, length capped) while the
  `Dess` suffix is always preserved. On a nick collision (433) the client
  retries with a disambiguator that **keeps `Dess` terminal**
  (`Baughn2Dess`), because the suffix is how *other* bridges recognize
  and de-duplicate it.
- **Outbound.** Only the local user's own chat messages are sent --
  tapped at the same `Mutation::Chat` site that feeds the synced chat, so
  events, subtitles, and narrator/system lines are never forwarded. A
  `/me` action goes out as a real IRC CTCP ACTION (the wire form is
  identical to DessPlay's inline `"\x01ACTION …\x01"`, so it forwards
  verbatim). Long plain lines are split to fit IRC's 512-byte limit; newlines
  become separate messages. A `/me` **CTCP action is never split** -- chunking
  it would break the `\x01` framing or emit several separate emotes for one
  action, so an over-long emote is left to the server's 512-byte truncation
  (the conventional client behavior); intentional. `/summon`'s ping is the
  one other outbound message and is **not** tapped from `Mutation::Chat`
  (it addresses specific nicks, not a broadcast to the group) -- it goes
  out directly as a PRIVMSG and is never mirrored into the local chat log
  or synced; only the summon *outcome* (who was pinged) becomes a local
  system line.
- **Inbound.** Messages from IRC nicks that do **not** end in `Dess` are
  shown locally, rendered like normal chat (per-nick color, mention
  highlight) but with a dim `irc` tag so they aren't mistaken for
  DessPlay peers. These lines are **local-only, never synced** -- each
  client runs its own bridge, so syncing them would duplicate. Messages
  from `*Dess` nicks are dropped: those are other bridges echoing DessPlay
  users who are already present via CRDT sync. (Heuristic cost: a genuine
  IRC user whose nick ends in "dess", e.g. `Goddess`, is also dropped --
  accepted, since the actor deliberately doesn't hold the roster.)
- **Lifecycle.** A dedicated [IRC actor](architecture.md#ircactor) owns
  the TLS connection, reconnects with capped backoff, answers PING, and
  is reconfigured live when the IRC settings change (disabling it makes
  it QUIT and idle). Connect/disconnect post local system lines. The
  channel is **public and unauthenticated** -- unlike the encrypted QUIC
  group, anything said in DessPlay chat is visible (and bot-loggable) on
  IRC; the settings screen says so.

### System Messages

The chat log narrates what the group is doing -- who joined, who paused,
what got put on -- so a glance at the chat is a glance at the session.
These lines are **derived, not synced**: the underlying facts already live
in the synced CRDT state or in the server's `PeerList` (presence). A small
synchronous **chat narrator** in the session layer diffs each new (state
view, peer list) against the previous one and emits a local system line
for each change. Because every client diffs the *same* synced inputs,
every client narrates the *same* lines -- consistent without any extra
wire traffic.

The cost of deriving rather than syncing is that **a late joiner does not
see past events**: a transition like "Baughn paused" cannot be
reconstructed from a snapshot that holds only the *current* value. That is
acceptable -- system lines are a real-time "what's happening now" cue, and
the durable answers live elsewhere (the Users pane shows who is present
now; the playlist pane shows the full play history in muted colors). The
two things that *do* reach late joiners are called out below: the player
crash (a real synced chat message) and the day separators (recomputed
from the persisted chat timestamps).

System lines render like the existing local-only lines: dim, no sender,
interleaved into the chat by shared-clock arrival time (the same
mechanism that already posts command feedback and archive results).

| Event | Derived from | Example line | Delivery |
|-------|--------------|--------------|----------|
| **Player crashed** (died twice in 30s) | the crashing client writes a chat message | "Baughn: my player crashed -- pausing" | **Synced** (a real chat message: persisted, shows the sender, late joiners see it) |
| **Player gave up** (died three times in 30s) | the crashing client writes a chat message | "Baughn: my player keeps crashing -- giving up until someone picks another file" | **Synced** (a real chat message; same rationale as Player crashed) |
| **Seek** (> 5s jump) | seek-authority + the authority's position | "Baughn skipped 08:12 → 12:34" (from = the previous sample extrapolated to the seek) | Local (only the *second and later* seeks in an episode -- the first is a Server->User authority flip with no prior sample to diff, and following the leader for a baseline would emit false jumps; intentional) |
| **New file** (manual select) | now-playing register change, no watched flip | "Now playing: [Frieren] - 02.mkv" | Local (the *what* persists in the playlist pane) |
| **New file** (EOF advance) | now-playing change + prior file's watched flag set | "Up next: [Frieren] - 02.mkv" | Local (ditto) |
| **Joined** | `PeerList`: a peer becomes Present | "Nero joined" | Local |
| **Connection lost** | `PeerList`: a peer becomes Lost | "Nero's connection dropped -- everyone paused" | Local |
| **Left** | `PeerList`: a peer becomes Departed (a timeout *or* a graceful `Goodbye`, which now departs in place) | "Nero left" | Local |
| **Back** | `PeerList`: Lost -> Present | "Nero is back" | Local |
| **Paused** | manual-override map: None -> Paused | "Baughn paused" — or "Baughn is not ready" when nothing was actually playing (pause/unpause words are reserved for real video stops/starts) | Local |
| **Resumed** | manual-override cleared (Paused -> None) | "Baughn unpaused" — or "Baughn is ready" when others still block (playback then starts on the last blocker's clear, which narrates the "unpaused") | Local |
| **Away** | manual-override map -> Away | "Kim is away" / "Baughn marked Kim away" | Local |
| **Not watching** | series-preference map -> NotWatching (now-playing series) | "Kim set to not-watching Frieren (by Kim)" | Local |
| **Watching (committed)** | series-preference map -> Watching (now-playing series) | "Kim is committed to Frieren (by Kim)" | Local |
| **Maybe** | series-preference map -> Maybe (now-playing series) | "Kim set Frieren to maybe (by Kim)" | Local |
| **Acknowledged absent** | `acknowledged_absent` gains `(now-playing, user)` | "Playing past Baughn (committed, away)" | Local |

**Attribution.** The resolved `StateView` does not record *who* wrote a
register, so attribution comes from the data, not the writer. The subject
of a per-user / per-`(user, series)` change is the map *key*; Away carries
its setter in the value (`ManualState::Away { set_by }`), so it can name
both when one user marks another ("Baughn marked Kim away"). The
**now-playing** writer is *not* recoverable (manual selection takes no
seek authority), so new-file lines are un-attributed; EOF-advance is told
apart by the prior file's watched flag flipping true. Watch-preference
lines are scoped to the **now-playing series** (the `/watch` / `/maybe` /
`/skip` / Ctrl-R surface), which keeps the List's bulk auto-writes for
other series out of the chat; the series-preference value itself carries a
`set_by: Option<UserId>` (mirroring `ManualState::Away`) for exactly this
reason -- `None` for every self-directed write and system auto-write
(rendered as the subject, "(by Kim)"), `Some(actor)` for a write targeting
*another* user (`n` on the Users pane, `/skip <name>` -- see
[User States](#user-states)), rendered as the real setter ("(by Baughn)").

**No cascade spam.** A single user-meaningful action often writes several
registers at once -- pressing play clears the manual override *and* sets
intent to Playing; an EOF advance moves now-playing, forces intent to
Paused, *and* sets the watched flag. The narrator emits **one** line per
action, not one per register. In particular, the server-forced intent ->
Paused on Lost / departure / EOF is never narrated as a bare "paused" --
it is already explained by the corresponding lost / left / new-file line.
Brief presence glitches under 30s never reach Lost, so they stay silent.
Drift-correction slews and the < 100ms ignore band never write the
seek-authority register, so they never produce a "skipped to" line; the
1500ms seek debounce already coalesces scrubbing into a single write.

**Seeders** are excluded from every presence-derived line, exactly as they
are excluded from the Users pane and playback gating.

**Day separators.** Watch parties straddle midnight, so the log marks each
new day -- but on a **biblical day boundary at 09:00 local time**, not
literal midnight (the small hours still belong to last night's session).
This is purely a **view concern**, not an event and not stored anywhere:
when rendering the chat, a separator ("──── Thursday, June 18 ────") is
inserted between two adjacent lines whose 09:00-anchored local date
differs. Because it is recomputed from the (persisted) chat timestamps, a
late joiner sees the separators too, and days with no messages produce no
separator. The boundary is local-time and per-client by design -- it is a
reading aid, never synced.

### Watching a Series

Typical evening flow:

1. Launch DessPlay, it connects automatically
2. Check the Series pane -- The List shows the group's ongoing shows and the
   next episode for each
3. Select series, add next episode to playlist (often already pre-fetched
   from the seeder)
4. Wait for friends' names to turn green
5. Anyone presses play, episode starts
6. Chat during the episode (appears on video)
7. Episode ends, add next one (or it's already queued)
8. Repeat until bedtime

---

## Client Roles

A client runs in one of two roles, selected via `--seeder`:

- **Interactive** (default): the full experience -- TUI, player, a human.
- **Seeder**: headless. No TUI, no player, no user states. Runs the sync,
  network, and file actors only.

The role is declared in the `Auth` message, so the server and all peers know
which clients are seeders.

### Seeder Behavior

- **Never gates playback.** Seeders are excluded from user-state derivation,
  the ready-state display, and presence-based pause rules. The Users pane
  shows them on a separate dim line ("seeders: nas").
- **Auto-fetches everything.** A seeder downloads every playlist entry as it
  is added (unwatched entries first, in playlist order), making it the durable
  seed for the group: whoever adds a file can go offline once the seeder has
  it. The primary seeder is colocated with the rendezvous server, so serving
  a file costs one trip over the NAS uplink per recipient -- the relay-only
  transfer design depends on this arrangement being the common case.
- **Indexes its library daily.** Like every client the seeder maintains a hash
  index of its media roots (see [Media Library Scanning](#media-library-scanning))
  and feeds new hashes into the `lookup_requests` set, contributing its (often
  large) collection to the group's browsable library. Because its store is big
  and stable it rescans once a day, not once a minute.
- **Storage** follows the same cache-retention setting as interactive clients
  (see [Download Cache](#download-cache-and-retention)). A NAS seeder sets
  retention to `infinite`; "should this be archived into the media library?"
  remains a manual, human decision via the archive action on any interactive
  client that shares the filesystem -- or simply by moving the file.
  A seeder persists no *settings* (it is configured by flags/env), but it
  **does** persist operational state — the hash cache and cache
  bookkeeping — in a database: a seeder may hold terabytes, so re-hashing
  its store on every startup is a nonstarter. On restart it re-discovers
  everything it already has (cache-hit, no re-hash) via the same
  **download-cache reconciliation** every client runs (see [Download
  Cache](#download-cache-and-retention)): the cache is hash-addressed, so
  prior downloads are resolved by hash, not by a media-root filename scan.

Multiple seeders are fine; they are ordinary peers in the file transfer
protocol. There is no special pairing between a seeder and its owner.

---

## Presence

Presence is the server's view of which clients are connected and responsive.
It is **not** CRDT state -- the server tracks it directly and pushes updates
in the `PeerList`. Presence is an explicit input to playback gating: rules
quantify over *present* users only.

A user's presence degrades in three stages:

| Stage | Trigger | Effect |
|-------|---------|--------|
| **Present** | Normal operation | Counted in playback gating |
| **Lost** | 30s without traffic (QUIC idle timeout; clients keep-alive every 10s, and position updates double as liveness) | Everyone pauses (server forces playback intent to Paused); system message in chat |
| **Departed** | 60s without traffic | Removed from gating and from the active Users list (shown on the dim known-offline line -- see below). Playback **stays paused** -- the intent register holds Paused until a human presses play; the usual response is to switch shows. No auto-unpause. |

Additional rules:

- **Brief glitches (< 30s) are invisible.** Everyone keeps watching; the
  shared clock keeps players aligned, and slew correction absorbs small drift
  on recovery.
- **Graceful quit** (`/quit`, Ctrl-C): an *immediate departure* -- the
  user goes straight to **Departed** (skipping the 30s Lost / 60s ladder),
  but stays listed exactly like a peer that timed out: shown on the dim
  known-offline line, and -- if **committed** (Watching) to the now-playing
  series -- still gating until the group acknowledges past them. Skipping
  the Lost stage is the *only* thing a clean quit buys over a silent
  disconnect; it does not waive a commitment (design's User States: the
  group waits for a committed user even when absent, "Lost, Departed, or
  quit"). If playback was running, it pauses (server sets intent to
  Paused) -- leaving mid-episode should not be silent -- and the server
  reclaims seek authority at once (a clean quit is final, unlike a Lost
  that may recover). At session end this is a no-op.
- **Return**: a reconnecting user re-enters as Present, syncs state, and is
  gated normally again. Playback does not auto-resume (intent is still
  Paused from when they were Lost).
- **Seek authority**: if the current seek authority becomes Departed, the
  server takes seek authority.
- Departed users' CRDT state (manual override, file availability) persists
  but is ignored by gating until they return.

**Known but offline (#15).** The above stages only cover peers the server's
in-memory registry has seen *this process lifetime* -- a user who hasn't
connected since the last server restart (or hasn't launched yet today) is
invisible to it. The server also persists a small `known_users` table
(username, last-seen millis), updated on every connect and disconnect, and
pushes it alongside `PeerList` as `known_offline: Vec<KnownUser>` (everyone
known who isn't currently Present, within a 30-day window). The Users pane
renders this as a single dim + italic list -- replacing the old plain
"offline" line -- showing "Kim (last seen 3d ago)" for both a user who left
minutes ago and one who hasn't shown up today, unified because both are
equally valid `n` / `/skip <name>` targets (the point of #15: rule on
someone's commitment without waiting for them to reconnect). A committed
(Watching) absent user is excluded from this list and shown instead as the
red "committed, away" blocker row, exactly as before.

---

## The List (Series Tracker)

The group's shared tracking spreadsheet, ported into the app: an explicit,
permanent record of what the group plans to watch, is watching, and has
finished. This is **separate from the playlist** (which holds concrete files
for a session); List entries are series-level and linked to playlist activity
through AniDB series IDs.

### Schema

```rust
struct SeriesListEntry {
    name: String,
    nero_name: Option<String>,        // Nero's alternative title; mandatory culture
    genre: Option<String>,
    notes: Vec<String>,               // free-form notes columns
    recommender: Option<String>,
    status: ListStatus,
    status_note: Option<String>,      // drop reason, hiatus progress, etc.
    source: Option<String>,           // where files come from; None = SubsPlease/batch
    watchers: BTreeSet<UserId>,       // who watches this series
    anidb_series_id: Option<AniDbSeriesId>,  // linked manually after import
}

enum ListStatus {
    ShortList,      // up next, high priority
    Planned,        // general plan-to-watch
    Active,         // currently being watched
    CurrentSeason,  // airing now, weekly episodes
    Waiting,        // waiting for release (movie, next season)
    Hiatus,         // paused, may resume ("Refresh / Haitus")
    Finished,
    Dropped,
}

// Tracked separately from the entry (different write pattern -- updated
// weekly/per-episode, including automatically):
struct NextEpState {
    next_ep: Option<String>,  // free text: "12", "S3-05", "Sisters", "movie 5?"
    available: bool,          // this week's episode is out (the ✓/✖ column)
}
```

`next_ep` is free text by necessity -- real entries include season prefixes,
OVA names, and guesses. When an entry is AniDB-linked and `next_ep` is
numeric, the server auto-advances it when the group finishes the matching
episode (same EOF transition that marks the playlist entry watched), and
resets `available` to false -- the new next episode is presumably not out
yet. Otherwise `available` is maintained by hand; automating it (e.g. via
AniDB episode air dates) is future work.

The List is never pruned: it is a few hundred rows of text, and the history
is the point.

### UI Integration

The Series pane gains **The List** as a third mode (alongside Recent Series /
All Series), grouped by status: CurrentSeason and Active first, then
ShortList, Planned, Waiting, Hiatus, with Finished/Dropped collapsed at the
bottom. Pressing `Enter` on a linked Active/CurrentSeason entry jumps
straight to the `next_ep` file: into the episode browser with the cursor on
that episode if anyone has it, so queueing tonight's episodes is a couple of
keypresses.

Entries display name, nero_name, next_ep (with an "out" marker from
`available`), and watcher initials. Editing fields and adding entries happens
in a small edit modal; linking an unlinked entry (`l`) opens the AniDB
search modal: it pre-searches for the entry's name (informal names like
"GochiUsa" resolve through the titles dump's synonyms), the user picks
from the ranked candidates and confirms. Enter on fresh results links;
editing the query re-arms search.

The `watchers` set wires into the per-series watch preference, and is the
declarative route to **commitment**: when an entry is linked, users *in*
the watchers set get `SeriesWatchState::Watching` (committed -- the group
waits for them even when absent) and users *not* in it get
`SeriesWatchState::NotWatching` (so they never gate playback on shows they
skip). Series with no List opinion stay at the **Maybe** default. Two
guards: an *empty* watchers set means "unrecorded", not "nobody", and
never writes preferences; and an existing preference (a manual choice) is
never overridden.

### Import

A one-shot `dessplay import-list <csv>...` subcommand imports the spreadsheet
CSVs:

- Section header rows within sheets ("Current Season", "Waiting", "Short
  List", "General", "Refresh / Haitus") set the status of following rows.
- The finished/dropped sheet maps to Finished, or Dropped when a field
  matches /abandon|drop/i (the data is messy; the importer prints a summary
  of every row it was unsure about for manual cleanup afterward).
- Watcher initials map to usernames via a flag:
  `--watchers B=Baughn,N=Nero,Q=Quickshot,D=Dagger,K=Kim`.
- AniDB linking is *not* part of import; entries arrive unlinked and are
  linked lazily from the UI.

---

## TUI Layout

### UI Principles

**No silent long-running work.** Any operation that can take more than
a moment (hashing a file for the playlist, scanning media roots,
downloading from peers, archiving) must show visible progress in the
UI while it runs — a user who sees nothing happen assumes nothing is
happening, and retries. Playlist-add hashing shows a centered progress
overlay (one bar per in-flight file); it is visually modal but captures
no input, so chat keeps working underneath. Phase 9's transfers reuse
the same pattern.

```
+----------------------------------+------------------+
|                                  | Recent Series |  |
|                                  | All Series       |
|          Chat Window             | (dual-mode,      |
|                                  |  franchise list)  |
|                                  +------------------+
|                                  | Users            |
|                                  | (colored by      |
|                                  |  ready state)    |
+----------------------------------+------------------+
|                                  | Playlist         |
|          Chat Window             | (current +       |
|          (continued)             |  previous in     |
|   [always-visible input line]    |  muted colors)   |
+----------------------------------+------------------+
|  Player Status: [=====>       ] 12:34 / 24:00       |
|  Now Playing: [Frieren] Sousou no Frieren - 01.mkv  |
+-----------------------------------------------------+
| Tab Next pane | Enter Send | Esc Clear | Ctrl-C Quit |
+-----------------------------------------------------+
```

**Proportions:**
- Bottom: Player status (3 lines) then keybinding bar (1 line)
- Left 50%: Chat (with input line at bottom)
- Right 50%, top: Series (three modes: Recent Series / All Series / The List)
- Right 50%, middle: Users
- Right 50%, bottom: Playlist

**Subtitle display (optional):** the local player's subtitles can be
surfaced in three modes, cycled live with `F2` (Off -> Intermixed ->
Separate pane -> Off) and persisted as a setting. The choice is **local
only** -- never synced (different releases / sub tracks per user are
expected). See [Subtitle Display](#subtitle-display) and
[Player Integration](#player-integration).

**Keybinding bar:** 1-line context-sensitive bar at the very bottom. Shows
available actions for the currently focused pane. Derived automatically from
the active component's keybinding declarations (see [ui-architecture.md](ui-architecture.md)).

**Focus cycling:** `Tab` cycles through Chat, Series, Users, Playlist

**Mouse support:** Click to focus panes, scroll, select items (if convenient to implement)

### Keyboard Shortcuts

| Key | Context | Action |
|-----|---------|--------|
| `Ctrl-C` | Any | Quit |
| `Ctrl-R` | Any | Toggle your own ready/unready (clears a manual pause; flips the now-playing series from NotWatching back to Maybe -- does **not** commit to Watching) |
| `Tab` | Any | Cycle focus: Chat -> Series -> Users -> Playlist -> Chat |
| `Tab` | Chat | Complete a username if the end of the input is a prefix of one (see below); otherwise cycle focus |
| `F2` | Any | Cycle subtitle mode: Off -> Intermixed -> Separate pane (persisted) |
| `F3` | Any | Open the settings screen (also `/settings`) |
| `Enter` | Chat | Send message (or execute `/command`) |
| `Esc` | Chat | Clear input |
| `Backspace` | Chat | Delete character before cursor |
| `Delete` | Chat | Delete character after cursor |
| `Ctrl-W` / `Ctrl-Backspace` / `Alt-Backspace` | Chat | Delete word before cursor |
| `Left` / `Right` | Chat | Move cursor |
| `Ctrl-Left` / `Ctrl-Right` (or `Alt-`) | Chat | Move cursor by word |
| `Home` / `End` (or `Ctrl-A` / `Ctrl-E`) | Chat | Move cursor to start/end of line |
| `m` | Series | Cycle mode: Recent Series -> All Series -> The List |
| `s` | Series (All mode) | Toggle sort: by title <-> by year |
| `/` | Series (Recent / All) | Start filtering franchises by title (removes Recent's watched-only default) |
| _printable_ | Series (filtering) | Add to the filter text |
| `Backspace` | Series (filtering) | Delete a filter character; on an empty filter, exit filtering |
| `Esc` | Series (Recent / All) | Clear the filter (and exit filtering) |
| `PgUp` / `PgDn` | Series | Move the selection by a page |
| `Enter` | Series | Browse franchise (episode browser or file browser) |
| `Enter` | Series (List mode) | Jump to next episode / open entry |
| `e` | Series (List mode) | Edit entry (modal) |
| `l` | Series (List mode) | Link entry to AniDB (search modal) |
| `Enter` | Episode Browser | Select season (cursor on its first unwatched row) / choose an episode or copy; no-op on a header row |
| `w` | Episode Browser | Cycle the selected file's group watched flag; no-op on a header row or in the season list |
| `PgUp` / `PgDn` | Episode Browser | Move the selection by a page |
| `Esc` / `Backspace` | Episode Browser | Go back (episodes -> seasons -> close) |
| `Enter` | File Browser | Open directory / choose file (add or map) |
| `Backspace` | File Browser | Up one level (from the roots listing, close); while searching, delete a search character |
| `Esc` | File Browser | Cancel; while searching, clear the search |
| _printable_ | File Browser (add / map) | Type-to-search the library recursively (root-relative paths, directories first); not in the directory picker |
| `PgUp` / `PgDn` | File Browser | Move the selection by a page |
| `s` | File Browser (directory picker) | Select the current directory |
| `a` | Users | Mark selected user as Away (or clear an Away you set) |
| `n` | Users | Mark selected user NotWatching for the now-playing series (works on a known-offline row too) |
| `Enter` | Playlist | Play selected entry (or open file browser on [Add New]) |
| `a` | Playlist | Add file (insert after selected entry) |
| `d` | Playlist | Remove selected entry |
| `w` | Playlist | Cycle the selected entry's series watch state: Watching -> Maybe -> NotWatching |
| `J` / `K` (or `j` / `k`) | Playlist | Move selected entry down/up (cursor follows the entry) |
| `A` | Playlist | Archive selected cached file into the download root |
| `M` | Playlist | Manually map selected entry to a local file |

Note: there is no `q` to quit -- too easy to hit while typing in chat.

---

## Network Protocol

### Overview

- **Transport**: QUIC for all communication
- **Topology**: Hub-and-spoke for everything -- state sync and file transfer
  both flow through the server (file transfer as relayed peer messages; there
  are no client-to-client connections)
- **Sync model**: `crdts` crate CRDTs synced via server -- see [sync-state.md](sync-state.md)
- **Wire protocol**: QUIC streams + datagrams, postcard serialization -- see [network-design.md](network-design.md)

### Rendezvous Server

Runs on the NAS (home server, NixOS; reachable at dessplay.brage.info,
unmetered 250Mbit up / 5Gbit down, dual-stack). Separate binary
(`dessplay-rendezvous`), colocated with the primary seeder process, which
connects over loopback. Responsibilities:

1. **Central coordinator**: All state sync flows through the server
2. **Peer list distribution**: Clients receive list of other peers,
   including role and presence state
3. **Presence tracking**: Detecting lost/departed clients and pushing updates
   (see [Presence](#presence))
4. **Relay**: Forward all file transfer traffic between peers (there are no
   client-to-client connections; see [network-design.md](network-design.md))
5. **Compaction**: Scheduled daily (default 12:00 UTC, `--compact-at`, configurable
   -- chosen to be far from watch-party hours). Compacts state, increments the
   epoch, and broadcasts the fresh snapshot to all connected clients, which
   adopt it like a stale-epoch reconnect. See [sync-state.md](sync-state.md).
6. **AniDB lookups**: Enriching playlist items with series/season/episode
   metadata, and fetching the relations graph for franchise grouping
7. **Authoritative actions**: EOF -> next file transitions, server actor ID for
   seek authority, watched flags, List next-episode auto-advance

**Authentication**: Password entered on first client launch, sent in plaintext
over TLS-encrypted QUIC. Server configured via `--password-file` or env var.

**TLS**: TOFU (Trust On First Use) -- server generates a persistent self-signed
cert; clients store and verify the fingerprint on subsequent connections.

### Time Synchronization

NTP-like protocol to establish shared clock:

1. Client sends timestamp `t1`
2. Server responds with `t1`, server time `t2`, response time `t3`
3. Client receives at `t4`
4. Calculate offset and round-trip time
5. Repeat periodically to maintain sync

All state timestamps use this shared clock.

### State Sync Protocol

Full details in [sync-state.md](sync-state.md). Summary of replicated data types:

| Data | CRDT Type | Notes |
|------|-----------|-------|
| Playlist | `Map<Ed2kHash, LwwCell<Option<PlaylistFileState>>>` | `Identifier`-based ordering; includes size and duration; `None` = removal tombstone (purged at compaction) |
| Watched flags | `Map<Ed2kHash, LwwCell<bool>>` | Server-only writes (at EOF, or a manual `MarkWatched` request from the episode browser) |
| Now Playing | `LwwCell<Option<Ed2kHash>>` | Standalone register; server writes on EOF |
| Seek Authority | `LwwCell<SeekAuthority>` (`Server \| User(UserId)`) | Standalone register; last seeker is position authority |
| Playback intent | `LwwCell<PlaybackIntent>` (`Playing \| Paused`) | Standalone register; users write on play/pause, server forces Paused on lost/graceful-quit/EOF-advance (not on the timeout-ladder Departed promotion -- already paused at Lost) |
| Series preference | `Map<(UserId, AniDbSeriesId), LwwCell<SeriesPreference>>` | Compound key; `SeriesPreference { state: Watching \| NotWatching \| Maybe, set_by: Option<UserId> }`, absent entry = Maybe; any user may write (design.md #7/#13) |
| Manual override | `Map<UserId, LwwCell<Option<ManualState>>>` | Per user; Away writable by anyone |
| Acknowledged absent | `GSet<(Ed2kHash, UserId)>` | Per-file one-shot: play past a committed-absent user; cleared on compaction |
| File availability | `Map<(UserId, Ed2kHash), LwwCell<FileAvailability>>` | Compound key |
| AniDB metadata | `Map<Ed2kHash, LwwCell<Option<AniDbMetadata>>>` | Server-authoritative; spans every file any client has indexed, not just playlist entries |
| File catalog | `Map<Ed2kHash, LwwCell<FileCatalogEntry>>` | Server-authoritative; filename + size from the lookup request, duration filled lazily; lets a client add a file it doesn't hold |
| Series relations | `Map<AniDbSeriesId, LwwCell<SeriesRelations>>` | Server-authoritative; franchise graph |
| The List | `Map<ListEntryId, LwwCell<SeriesListEntry>>` | Any peer; never pruned |
| List next-ep | `Map<ListEntryId, LwwCell<NextEpState>>` | Any peer; server auto-advances |
| Lookup requests | `GSet<FileHashInfo>` | Clients insert (playlist + library scan); cleared on compaction |
| Chat | `GList<ChatMessage>` | Grow-only ordered list; trimmed on compaction (server archives full history) |
| Playback position | `Map<UserId, LwwCell<PlaybackPosition>>` | Per user, high frequency, datagram-only transport |

All registers are `LwwCell<V>` — DessPlay's own max-merge LWW register
(`crdts::MVReg` proved non-convergent inside `Map`; see sync-state.md).
`ActorId` type parameters omitted from the table for brevity -- all CRDTs
use `ActorId` as the actor type. See [sync-state.md](sync-state.md) for
the full `Lww<V>` design.

Whether the video actually plays is **derived**: it plays iff playback
intent is Playing and no interactive user blocks. A NotWatching or Away
user never blocks; a **Maybe** user blocks only while present and not
ready-to-play; a **committed** (Watching) user blocks whenever not
ready-to-play, *including while absent*, until they return or the group
acknowledges past them for the current file. The intent register exists
because gating alone cannot express "stays paused after the blocker
departs" -- see [Playback Rules](#playback-rules).

### Chat Protocol

Chat messages include:
- Sender username
- Message text
- Timestamp

Reliability: Chat is a `crdts::GList` (grow-only list). New messages are
sent through the server; the CRDT handles ordering and deduplication.
The [system messages](#system-messages) are local-only and derived
per-client, *not* in this GList; the one exception is the player-crash
notice, which the crashing client writes as an ordinary chat message (so
it persists and reaches late joiners).

---

## File Management

### Media Roots

User configures a list of directories to search for media:

```
media_roots = [
    "/home/user/anime",
    "/mnt/nas/shows",
    "/home/user/Downloads"
]
```

Stored in local SQLite database, editable via settings screen.

### Media Library Scanning

Clients keep a **library index** of every file under their media roots, so the
franchise browser can show the group's collective collection and add any of it
-- not just files already in the playlist. The index reuses the `hash_cache`
table (path -> ed2k root + per-block hashes, keyed by `(mtime, size)`); it is
no longer filled only on demand.

- **At startup** the client walks every media root, `stat`s each file, and
  hashes anything new or changed (a path missing from `hash_cache`, or whose
  `(mtime, size)` disagrees). Unchanged files are a cache hit -- no re-read.
- **Periodically** the client re-walks the roots and re-hashes only changed
  files. Interactive clients rescan about once a minute; a seeder, whose store
  is large and stable, rescans once a day.
- **Hashing yields to transfers (#21).** Scan hashing is bulk disk work
  with no deadline, while transfers are latency-sensitive (a source that
  serves nothing for 30s is snubbed) — so while transfer traffic (serving
  or downloading) is active, scan hashing defers, resuming ~10s after the
  traffic goes quiet. The walk itself (stat-only) still runs.
- **The walk also prunes**: an index row whose file has vanished from under
  the roots (moved or deleted behind the app's back) is removed — the disk
  is the truth, the index follows it. Without this a moved file kept its old
  row forever, leaving ghosts in everything built on the index (the file
  browser's search and anchor placement). Rows outside the roots (the
  download cache) are reconciled by their own startup pass, not the scan.
- For every indexed hash that lacks metadata in the synced state, the client
  inserts a `FileHashInfo` (hash, size, filename, mtime, and a title-like
  containing-directory `series_hint`) into the
  `lookup_requests` GSet -- the same "please look this up" set the playlist
  uses, now fed by the whole library. The scan has each file's path and mtime
  in hand (the mtime keys the `hash_cache`; the path yields the directory
  hint), so library requests always carry both. Server-side per-hash de-duplication and the cross-client
  "already checked" bookkeeping (the `anidb_queue` table) keep AniDB load
  bounded even when several clients index overlapping collections.

Active hashing surfaces progress like any other long-running work (see
[UI Principles](#ui-principles)); a quiet rescan that finds nothing new is
silent. Since this populates `hash_cache` ahead of time, resolving a file on
playlist-add is usually an instant cache hit rather than a fresh hash.

This is what makes "add a file you don't have" possible: the server records
each looked-up file's identity (filename + size, from the lookup request) in
the broadcast **file catalog**, so a client that has never held the file can
still construct a playlist entry for it and download it. See
[Lookup flow](#parsing-files-to-seriesseasonepisode) and the file catalog in
the [State Sync](#state-sync-protocol) table.

### File Matching

When a playlist item is added:

1. Extract base filename (e.g., "Frieren - 01.mkv")
2. Search all media roots recursively for exact filename match
3. If found: store local path
4. If not found: mark as missing (red in UI)

The adder fills in the entry's `size_bytes` and `duration_millis` (they have
the file, so both are cheap to read). Size lets downloaders compute chunk
counts; duration drives the bitrate-based unpause rule and the watched
threshold for files still downloading.

### Download Cache and Retention

Files retrieved from peers are written to the **download cache**
(`$XDG_CACHE_HOME/dessplay/files/`), **hash-named** (`<cache>/<ed2k-root>`).
They are never automatically promoted into a media root.

**The cache is hash-addressed, and the filesystem is the source of truth.**
The `cache_entries` table is an index over the cache, not an authority: a
user may delete, move, or truncate files behind the app's back. So at
**startup the file actor reconciles `cache_entries` against disk** — a row
whose file is gone or whose size disagrees is pruned (along with its
`hash_cache` row), which makes the playlist entry re-resolve to Missing and
re-download; a surviving row is re-registered as a servable copy. Resolution
then finds a cached download **by hash** (it checks `<cache>/<hash>`
directly), because the cache is hash-named and the media-root *filename*
search can never match it. Two **runtime guards** cover deletions that
happen mid-session rather than between runs: a player load failure
(file gone under us) and a serve-time absence (a peer asks for a file we no
longer hold) both drop the local copy, prune its bookkeeping, and flip the
file to Missing so it re-resolves.

**Retention** (`cache_retention`, per client): a cached file becomes
*evictable* once it has been watched (85% rule) or sits behind the group's
progress in the playlist (watched flag set). An evictable file is deleted
`cache_retention` after its last access. Special values:

- `0`: deleted at the next eviction pass after watching -- the
  "small laptop" setting; nothing accumulates
- `infinite`: never deleted -- the NAS/seeder setting

Eviction passes run at startup and on EOF-advance. The now-playing file and
queued unwatched playlist entries are never evicted, regardless of retention.

**Archive**: an explicit action (`A` in the playlist pane) that moves a cached
file into `[Series name]/[Season #]/[Original filename]` under the download
root (the topmost media root). This is the deliberate "keep this in the
library" decision; retention is the default "it was just for the watch party"
path. *Implementation note (Phase 9A):* the destination is
`[Series name]/[Original filename]` — the `Season #` level is collapsed,
since AniDB models each season as its own anime (a franchise member), so a
single series name is already one season's folder. The series-name component
is sanitized to a safe path component.

Cache-only files (those with a download-cache row, i.e. not yet in a media
root) are flagged in the playlist pane with a dim right-aligned **`temporary`**
marker; `A` only acts on such rows. Archiving moves the file into the library,
so the marker clears — that is the success feedback. Both success and failure
also post a local-only system line to the chat pane ("Archived …" / "Archive
failed (…): …"); these notices are local, not synced.

**Pre-fetching**: clients with downloading enabled fetch playlist entries
*ahead* of now-playing (in playlist order) in the background, so next week's
episode is usually local before the session starts. Seeders fetch everything
(see [Client Roles](#client-roles)); interactive clients pre-fetch within
their retention/disk constraints. An interactive client **skips
auto-download** for entries whose series it has marked **NotWatching** --
no point fetching a show you've opted out of. **Maybe** (the default) and
**Watching** entries are prefetched normally; a NotWatching file that is
already local still loads (you can mute), it is just never fetched.

The **auto-download** setting (default on) is a coarser switch: turning it
off disables *all* automatic fetching for that client -- both the prefetch
window and the missing now-playing file -- making it a "bring your own
files" participant. A missing now-playing file from a **known** series
then stays Missing (obtain it via a media root or manual map); a missing
file from an **unknown** series resolves to **NotWatching** immediately
rather than waiting on a download that will never arrive. Seeders are
unaffected (they persist no settings and must seed the whole playlist).

### Parsing files to series/season/episode

We use the AniDB UDP API, with the understanding that the information may
be incomplete and/or require later updates.
See https://wiki.anidb.net/UDP_API_Definition

Crucially:
- The API is rate-limited. Clients MUST NOT send more than 1 packet every 2
  seconds, and also MUST NOT send more than 1 packet every 4 seconds with a
  burst of 60.
- Server-throttled packets are counted against this rate limit. Throttling is
  unpredictable; on a missing response, the client MUST wait 5 seconds before
  retrying.
- Files SHOULD be re-validated on a reasonable schedule: Every 30 minutes if
  it is less than a day old, every 2 hours if it's less than a week, and so on.
  Files older than 3 months do not need to be re-validated. This is only true
  when AniDB fails to return data for a file.
- Files which *do* have data should still be re-validated, but MUST NOT be
  re-validated more than once per week.
- The code needs to account for the client being turned off most of the time;
  the validation queue needs to be in SQLite, not done by way of sleeps.
- The client id is "dessplay".
- All commands besides LOGIN require first logging in.

All interaction with AniDB is done by the rendezvous server, not the clients.

**Credentials:** the server needs an AniDB account with the "dessplay"
client registered on it. Credentials arrive via `DESSPLAY_ANIDB_USER` /
`DESSPLAY_ANIDB_PASSWORD` (or `--anidb-user`/`--anidb-password`), never
persisted; with no credentials the integration is simply disabled. Note
the UDP API has no TLS: AUTH sends the password in cleartext UDP. The
account is used for nothing else, and AniDB's `ENCRYPT` command remains
future work.

**Lookup flow:**
1. Clients insert `FileHashInfo` (hash, size, filename, and -- when the
   requester holds the file locally -- the file's mtime and a title-like
   containing-directory name `series_hint`) into a
   `GSet<FileHashInfo>` -- a "please look these up" set -- for every file that
   lacks metadata, whether it is a playlist entry or just a file the
   [library scan](#media-library-scanning) found in a media root. Any client
   may request; the server deduplicates, and the GSet absorbs repeated inserts.
   This also re-arms requests after compaction clears the set. (A playlist
   entry a client doesn't hold has neither mtime nor directory hint; the
   request omits both.)
2. The server drains entries from this set into its AniDB lookup queue, and
   **records each file's identity** (filename + size, taken from the request)
   in the server-authoritative **file catalog** (see the
   [State Sync](#state-sync-protocol) table). The catalog is broadcast like any
   other server write and persists across compaction, so every client -- even
   one that has never held the file -- has enough to build a playlist entry for
   it. Duration is not in the request; it is filled lazily (by an owner once
   the file is held, or at download time) and only gates the bitrate-based
   unpause rule until then.
3. On success: server writes full `AniDbMetadata` (series name, ID, episode).
4. On failure (AniDB doesn't know the file): server writes filename-derived
   metadata once -- a later re-validation miss never clobbers real metadata.
   The fallback series name is the requester's `series_hint` (a title-like
   containing-directory name, e.g. `RahXephon` for a file under
   `<root>/RahXephon/Season 1/...`) when one was supplied, else the filename
   minus its extension; no series ID, no episode number. The directory hint
   keeps a series' AniDB-unknown episodes grouped into one franchise instead
   of one per episode -- without it, per-episode filenames each parse to a
   distinct series name. The hint is computed client-side by walking the
   ancestors between the file and its media root, skipping season/disc folders
   and generic containers (`Movies`, `Anime`, ...) and taking the first
   title-like directory; the server stores the first non-null hint reported
   on the `anidb_queue` row. The
   re-validation cadence is an age-based ladder for never-seen files
   (30 min if < 1 day old, 2 h if < 1 week, 12 h if < 30 days, 3 days if
   < 90 days, then never) and weekly for files AniDB knows. **The ladder's
   age is anchored on the *older* of when the server first queued the file
   (`first_seen`) and the file's mtime** (the minimum of the two; mtime
   absent falls back to `first_seen`). This keeps files owned for years off
   the aggressive new-file cadence: without it, a queue reset stamps every
   long-owned unknown file with a fresh `first_seen` and re-polls it every
   30 min indefinitely. Clients supply the mtime in the lookup request; the
   server stores it on the `anidb_queue` row, lowering it toward the oldest
   value reported (a request without an mtime never raises it).
5. Either way, the metadata register becomes `Some(AniDbMetadata)` --
   downstream code always has a series name to work with.

**Durability reconciliation:** the queue attempt (settled, re-check in a
week) is written to SQLite at once, but the metadata write lands only in
the periodically-snapshotted CRDT state. A restart in that window loses the
metadata yet keeps the settled queue row -- the file is then orphaned (no
metadata, no retry for a week). At **startup the worker reconciles**: any
`anidb_queue` row marked `has_data` whose hash has no metadata in the
loaded state is re-armed (due now, `has_data` cleared), so it is looked up
again. NoData rows self-heal on their short ladder and are left alone.

**Directory-hint reconciliation:** the fallback series name is written
once, at the first lookup, using whatever `series_hint` the `anidb_queue`
row holds then. But the hint can arrive *after* that write -- a playlist
add carries no hint (the client may not hold the file) and races ahead of
the hinted library scan -- so the first-seen episode of a series can be
frozen with its per-episode filename stem and split into its own franchise.
Each worker pass therefore reconciles: for every row with a learned
`series_hint`, if the file's metadata is filename-derived and its
`series_name` differs from the hint, the server rewrites it to the hint
(no AniDB call, independent of the settled lookup schedule). Real AniDB
hits are never touched, and a name already matching its hint is left alone,
so this quiesces.

CRDT types:
- Lookup requests: `GSet<FileHashInfo>` (cleared on compaction)
- File catalog: `LwwCell<FileCatalogEntry>` keyed by ed2k hash (server-only
  writes; filename + size from the request, duration filled lazily)
- Metadata: `LwwCell<Option<AniDbMetadata>>` keyed by ed2k hash (server-only writes)

See [sync-state.md](sync-state.md) for the full `AniDbMetadata` struct.

**Franchise relations:** grouping series into franchises requires AniDB's
relations graph (sequel, prequel, side story). When a file lookup yields a new
series ID, the server queues ANIME lookups for it and walks its relations
recursively (each hop is another rate-limited request, so the graph fills in
over hours -- fine, it's needed for browsing, not playback). Results are
cached in server SQLite and replicated as the server-authoritative
`SeriesRelations` map. Clients build franchise groupings from this map
(connected components over the relations graph); files without a series ID
group by parsed series name as a fallback. Manually linking a List entry also
seeds the walk for its series.

Only **structural** relation edges merge two series into one franchise:
sequel/prequel chains, alternative versions (remakes), and
side/parent/summary/full-story spin-offs (`RelationKind::groups_franchise`).
Crossover and shared-universe edges -- same setting, shared characters, music
videos, and AniDB's catch-all crossover code -- link *related but separate*
works and are deliberately ignored. Without this filter a single crossover
like *Isekai Quartet* (which relates to Overlord, KonoSuba, Re:Zero and Youjo
Senki) would collapse every show it touches into one giant component.

The relations walk pulls in the whole graph -- sequels you don't have,
standalone shows reached through a crossover -- so a series can exist purely
as a relation target with no associated file. Those are filtered from the
view: a franchise member with no known file is dropped from its season list,
and a franchise with no files at all does not appear. Title and year are still
derived from the full component, so "Overlord" stays the franchise name even
when only a later season is held.

**Name search (the AniDbSearch modal):** the UDP API has no
multi-result search -- `ANIME aname=` is an exact-title lookup, useless
for informal names. Instead the server fetches AniDB's daily
**anime-titles dump** (`anidb.net/api/anime-titles.dat.gz`, the
sanctioned approach; at most one download per day) into SQLite and
answers search requests locally: case-insensitive substring over all
titles and synonyms, ranked exact > prefix > substring, one hit per
series. Search requests/results are plain wire messages
(`AniDbSearch`/`AniDbSearchResults`), not CRDT state.

### Manual File Mapping

When explicitly invoked:

1. User selects the missing item in playlist
2. Opens file browser
3a. If series & episode number is known, and there is a local match (different
    filename, same series, season & episode): Cursor is placed on this file.
3b. If series is known, and the user has previously used the map function for
    this series: Browser opens to the most recently used directory.
3c. Otherwise, the browser opens to the list of media roots.
4. User selects correct file
5. Mapping stored locally

**Drag-in adoption.** A second, lower-friction route to the same outcome:
if the user loads a file **directly into the player** (drag-and-drop into
the mpv window) whose basename matches the now-playing entry, dessplay
adopts it -- it registers the loaded path as a manual mapping for the
now-playing file, exactly as if it had been picked through the browser.
This clears the Missing/placeholder state, flips `FileAvailability` to
Ready, and loads the real video. It is **filename-trusted, no hash
check** -- the same "the user explicitly chose this file" exemption the
browser map gets (see [Content Hash](#content-hash)). The trade-off is
that a same-named *different encode* dropped in would silently desync the
client from the group; this is accepted for parity with the browser map
and because the user deliberately loaded that exact file. dessplay learns
what mpv has loaded by observing its `path` property (see
[Events from Player](#events-from-player)); a path it never commanded, with
a matching name, is the trigger. (Especially handy in attach mode, where
driving mpv directly -- including dragging files in -- is the normal
workflow.)

### Content Hash

Before playback can unpause:

1. Compute ed2k hash
2. Compare with other Ready users

If hash mismatch: File State is set to Missing, cannot participate until resolved.

**Mismatch re-check (#26).** A name-matched file that fails the hash is
usually a copy or external download still being written into a media
root — the hash ran mid-write. The file actor watches such files: it
polls the path's `(mtime, size)` about once a second (a cheap `stat`),
and once the file has changed *since the failed hash* and then held
still for a couple of polls, it re-resolves — so the entry flips to
Ready seconds after the write finishes, not at the next library scan a
minute later. A mismatch that never changes (a genuine different encode)
is never re-hashed — its hash-cache row still matches the disk — and its
watch expires after 10 minutes; the periodic scan remains the long-tail
safety net.

This is skipped for manually-mapped files (user explicitly chose a different file).

### Watch Tracking

Two levels, deliberately distinct:

**Personal** (local SQLite, keyed by hash/series so it survives cache
eviction):
- A file is "watched" when 85% of its duration has been played
- Used for:
  - Sorting "Recent Series" (most recently watched on top)
  - Filtering "unwatched files" in series browser
  - **Known series detection**: a series is "known" if you have previously
    watched any file from it. This affects missing file behavior -- see
    [File Matching](#file-matching)

**Group** (synced state):
- The server sets the playlist **watched flag** at EOF, and auto-advances the
  List's `next_ep` for linked series
- Used for: play-history display (muted playlist entries), eviction
  ("behind the group"), and The List's "where are we?" answer -- so a user
  who misses a session still sees the group's position, not their own

### Placeholder Image

When a user is set to Not Watching due to a missing file, DessPlay generates a
PNG and loads it into the player. The image displays:
- The filename that's playing
- "You don't have this file"
- Current session status (who's watching, who's not)

This prevents the user from seeing a stale video frame or an empty player
window while others are watching.

---

## Player Integration

### Supported Players

- **mpv**: Primary, via IPC socket (JSON protocol), scripted for behavioural changes
- **VLC**: Via embedded Lua TCP script. Whether VLC support lands in dessplay
  v2 is an open scope decision (revisited in Phase 10); mpv is the primary
  target throughout.

Player choice is per-user configuration.

### Player Lifecycle

1. **Launch**: One persistent mpv instance per session (`--idle
   --keep-open`), spawned when the first file loads; later files are
   swapped in with `loadfile`. Files always open paused; the derived
   playback state then decides.
2. **Control**: Send play/pause/seek commands via IPC
3. **Monitor**: Read current position, playback state
4. **OSD**: Display chat messages in video window
5. **Crash handling** (also covers the user closing mpv by hand). The
   response escalates with the number of deaths in a row, each within 30s
   of the last:
   - **First death**: relaunch silently — reload the file, seek to the last
     position, restore the desired pause state.
   - **Second death within 30s**: *additionally* pause globally and notify
     in chat — the relaunch then comes up paused, the safe state if
     the file itself is crashing the player. Unlike most
     [system messages](#system-messages), this one is **shared**: the
     crashing client writes a real chat message (and forces playback
     intent to Paused). A crash is the one state change peers cannot
     derive from their own view (they have no signal for *another* user's
     player dying), so it must be communicated — and being an ordinary
     synced chat message, it persists and reaches late joiners.
   - **Third death within 30s**: stop relaunching. A file that reliably
     kills the player would otherwise loop forever (spamming the log and
     re-pausing on every death). The client stays paused and writes a
     second shared chat message ("my player keeps crashing — giving up
     until someone picks another file"). Loading a **different file** (a new
     now-playing) resets the counter and brings the player back — the
     deliberate recovery action. The crash counter resets whenever a
     different file is loaded, so deaths spaced further than 30s apart
     never accumulate toward the give-up threshold.

**Attach mode (`--attach-mpv=<socket>`).** A dev/headless aid for working
without a desktop (e.g. over ssh): instead of spawning mpv, dessplay
*attaches* to one the user already launched at a given `--input-ipc-server`
socket. The user runs mpv in a separate terminal (e.g. a tmux pane) with
`mpv --idle=yes --keep-open=yes --vo=tct --input-ipc-server=<socket>` — the
`--idle --keep-open` are required (the EOF/load mechanics depend on them) and
`--vo=tct` renders video as terminal cells. mpv accepts multiple simultaneous
IPC clients, so dessplay drives loads/seeks/observation while the user's
keyboard in that terminal still pauses and scrubs directly. A manual pause
there is an ordinary `pause` property change — indistinguishable from one in a
normal window — so the existing echo model propagates it to the group with no
special handling. dessplay **never `quit`s an attached mpv** on shutdown (it
isn't ours to kill); if that mpv dies, the relaunch path re-attaches, waiting
for it to come back. Interactive-only; seeders have no player.

### Commands Sent to Player

- `loadfile <path>`: Load video file
- `set_property force-media-title <name>`: Override the displayed title with
  the playlist filename (set before `loadfile`). Cache downloads are
  hash-named on disk, so without this mpv shows the ed2k hash instead of the
  real episode name.
- `pause` / `unpause`: Control playback
- `seek <seconds>`: Seek to position
- `set_property speed <factor>`: Slew playback rate for drift correction
  (±2% max; mpv's pitch correction makes this inaudible)
- `get_property time-pos`: Query current position
- `osd-overlay <id> ...`: Persistent OSD overlays — one slot for the
  rolling chat log (top-left, per-message 8s minimum retention), one for
  the "Waiting for …" blocker summary (top-right). Overlays stay until
  rewritten or cleared and are re-applied after a player relaunch;
  independent slots mean chat and the blocker line never clobber each
  other (the old timed `show-text` was a single slot and did)

### Events from Player

- Position updates (polled or subscribed)
- Pause/unpause events (distinguished: user-initiated vs programmatic)
- Seek events (distinguished: user-initiated vs programmatic)
- Subtitle text changes (observed `sub-text/ass-full` property; feeds the
  subtitle log, with the ASS speaker field for per-speaker coloring)
- Loaded-file path changes (observed `path` property; distinguished:
  our own `loadfile` echo vs. a file the **user** loaded directly, e.g.
  drag-and-drop into the mpv window). A user-loaded path whose basename
  matches the now-playing entry is **adopted** -- see
  [Manual File Mapping](#manual-file-mapping).
- EOF (file ended; reported to the server, which owns the transition)
- Load failure (the file could not be **opened** — gone, unreadable,
  undecodable). `loadfile` is accepted asynchronously, so this is *not* a
  command error: mpv reports it later as an `end-file` with `reason: error`.
  It flips the file to Missing and re-resolves (the
  [player-load-failure guard](#download-cache-and-retention)) — the path we
  held may be stale (the file moved between media roots). Without observing
  this event, dessplay would believe a load that mpv silently dropped and
  unpause on a file it never opened, showing only the forced media title.
- Exit (clean or crash)

The user/programmatic distinction is made on **our** side — mpv does not
flag event origins. The player actor tracks what it commanded and
swallows matching observations as echoes (architecture.md, PlayerActor);
because correction is observe-and-correct rather than locally enforced,
a misattributed echo self-heals on the next derived-state round trip. The
`path` observation uses the same model: an observed path equal to the one
we last commanded (including the placeholder PNG) is our echo and is
swallowed; any other is the user's.

### Subtitle Display

Subtitles are **local-only**: whatever line the user's own player is
currently displaying, appended to a rolling log. Nothing is synced --
different releases or sub tracks per user are fine. Image-based subtitle
formats (PGS/VobSub) expose no text; the log simply stays empty for those.

We observe mpv's `sub-text/ass-full` property (not plain `sub-text`):
it returns the full `.ass` event line, so the client can read the ASS
`Name`/actor (speaker) field for per-speaker coloring. The client strips
the ASS override tags (`{...}`, `\N`/`\h`) itself -- work mpv did for us
under plain `sub-text`. This requires **mpv >= 0.39.0** (where
`sub-text/ass-full` was added). Non-ASS formats (SRT) convert to an event
with an empty Name -- no speaker, no special color.

The log is surfaced in one of three modes (cycled with `F2`, persisted as
the `subtitle_mode` setting):

- **Off**: subtitles are not shown.
- **Intermixed**: subtitle lines are folded into the chat log, dim with a
  `»` marker, ordered by arrival. They share the chat's interleave domain.
  (No speaker coloring here -- the lines stay uniformly dim.)
- **Separate pane**: the chat area is split horizontally and the lower
  portion shows recent subtitle lines, **newest first (on top)** so the
  freshest line sits next to the chat input box just below it. Each line's
  text is **colored by its speaker** -- the ASS `Name` field hashed into
  the same name->color palette chat uses for usernames, so each speaker is
  visually distinct. The speaker name itself is **never displayed** (it is
  often a character name -- a spoiler); only its color is. The `MM:SS`
  timestamp prefix stays dim.

Each line carries the **in-video position** (mpv `time-pos` at the moment
the cue appeared), shown as its `MM:SS` timestamp -- *not* the wall clock.
Interleaving in Intermixed mode still orders by wall-clock arrival (the
chat domain); the displayed timestamp and the sort key are deliberately
two different clocks.

**Incremental ASS reveals and overlapping cues.** The on-screen cue-set
evolves while mpv re-emits the whole joined value on every change, so
consecutive observations are often the *same* utterance growing or
shrinking rather than a new line. Two cases:

- *Reveal*: some subs reveal a line letter-by-letter over 2-3s as
  rapid-fire cues, each a longer prefix of the last.
- *Overlap*: when two ASS events display at once mpv joins them (separated
  by a space); as one ends the combined text shrinks back to just the
  other. The disappearing event can sit at either end of the join (mpv's
  order is not fixed), so the shrink leaves a prefix *or* a suffix of what
  was shown.

The log collapses any such prefix/suffix relationship between the previous
line and the new text into one entry: a growth replaces it in place
(keeping the original cue's timestamp and tracking the latest speaker); a
shrink-back is dropped as a redundant re-show. Without the shrink case a
brief interjection overlapping a stable line produced a duplicate when it
cleared (SL2_Episode-141 at ~03:19: "…glory." re-appeared once the
overlapping "Coming!" ended). An exact repeat is the degenerate case and
collapses too. A multi-line cue arrives newline-separated; since the log
renders one line per cue, newlines become spaces (so a two-line cue reads
"you demons", not "youdemons"). Known limitation: an unrelated later cue
that happens to be a prefix or suffix of its predecessor will be collapsed
(rare; accepted -- no time-window guard).

---

## Data Storage

### SQLite Database

Location: `$XDG_DATA_HOME/dessplay/dessplay.db` (typically `~/.local/share/dessplay/`)

**Single-instance lock:** at startup a process takes an exclusive advisory
lock (`File::try_lock`) on `<db>.lock` and `<cache>/.lock` and refuses to start
if another instance already holds either — two processes sharing one db/cache
(e.g. a client and seeder from the same home dir) corrupt each other's state.
Run a second instance with its own `--db` and `--cache-dir`.

Uses `rusqlite` with `bundled` feature. CRDT state is persisted per-room as
periodic **full-state snapshots** (postcard blobs) so it survives full
disconnects; there is no persisted op log. On startup, the stored state is
loaded and passed to the sync engine as initial state. The current epoch is
also stored so the client can detect stale state on reconnection.

**Deliberate non-goal:** local ops the server has not yet seen are buffered
in memory only. A crash loses the most recent local edits — accepted, since
crashes should be rare enough not to matter, and an edit that *caused* a
crash should not be replayed into the next session.

**Settings** (username, server, password, media roots, player choice, cache
retention, upload limit, subtitle mode, auto-download, and the IRC bridge
settings -- enabled, server, TLS, channel) live in the same SQLite database
and are edited through the settings screen. The password is stored in plaintext
— consistent with the threat model below. Command-line flags and environment
variables override stored settings at runtime but are never persisted.
Seeders and the rendezvous server store no settings at all; they are
configured purely by flags/environment (systemd services on NixOS).

### Schema

Versioned via `PRAGMA user_version`; migrations are append-only. All
tables are `STRICT`. Timestamps are unix milliseconds, caller-supplied
(storage never reads the clock — keeps tests deterministic).

The postcard `crdt_state` blob has **no internal version tag**, so a
field added to `CrdtState` (e.g. `acknowledged_absent`) is decoded with a
small forward-compat fallback: try the current layout, and on failure
decode the previous layout (`CrdtStateV1`) and upgrade it (new fields
default-initialized). This matters most for the *server*, which is
authoritative and cannot re-sync its lost state from anyone; an
interactive client can fall back to dropping an unreadable blob and
re-syncing from the server. New struct fields are appended (never
inserted) so the previous layout is a strict prefix.

**Client** (`$XDG_DATA_HOME/dessplay/dessplay.db`):

| Table | Contents |
|-------|----------|
| `settings` | Key-value settings (username, server, password, player, ready_on_startup, cache_retention, upload_limit, subtitle_mode, auto_download, irc_enabled, irc_server, irc_tls, irc_channel) |
| `media_roots` | Ordered media roots; position 0 is the download target |
| `crdt_state` | Latest snapshot per room (epoch + postcard blob); single `'default'` room in v1 |
| `watch_history` | Personal watched files: hash → series id/name, filename, watched_at |
| `cache_entries` | Download-cache bookkeeping: hash → path, size, last_access; an index, reconciled against disk at startup (stale rows pruned) |
| `hash_cache` | Path → ed2k root + per-block hashes, keyed by (mtime, size); skips re-hashing unchanged files (Phase 9A); doubles as the **library index** populated by the periodic media-root scan; pruned alongside a removed cache entry |
| `manual_mappings` | Playlist hash → user-picked local path |
| `series_map_dirs` | Per-series last-used mapping directory (`anidb:<id>` / `name:<parsed>`) |
| `tofu_fingerprints` | Pinned server cert fingerprints; write-once (replacing requires explicit forget) |

**Server** (`$XDG_DATA_HOME/dessplay-rendezvous/rendezvous.db`,
`--db-path` to override):

| Table | Contents |
|-------|----------|
| `crdt_state` | The authoritative snapshot (epoch + postcard blob) |
| `chat_archive` | Full chat history, archived before compaction trims the replicated GList; unique on (timestamp, sender, text), mirroring GList dedup |
| `anidb_queue` | FILE validation queue: hash, size, filename, mtime (anchors the re-validation ladder on the file's real age), `series_hint` (title-like containing-directory name; the AniDB-miss fallback series name, so episodes group by folder), attempt bookkeeping, `next_attempt` scheduling (`i64::MAX` = settled tombstone) |
| `anime_queue` | ANIME (relations-walk) queue: aid, attempt bookkeeping; the graph fills in over hours and must survive restarts |
| `anidb_titles` | The anime-titles dump (aid, kind, lang, title); backs local name search |
| `kv` | Bookkeeping (e.g. the titles dump's last fetch time) |
| `known_users` | Every username ever seen, with a last-seen timestamp (design.md #15); updated on connect/disconnect, survives restarts unlike the in-memory peer registry |

---

## Security / Threat Model

Authentication uses a password entered on first launch and stored in the local
database. The password is used to authenticate with the rendezvous server.
Anyone with the password can connect. This is acceptable for the intended use
case (small friend groups) but should be documented clearly.

- **Identity**: Users are identified by self-chosen nicknames. There is no
  cryptographic identity -- users trust each other not to impersonate.
- **Confidentiality**: All traffic is encrypted via QUIC's built-in TLS 1.3.
- **Integrity**: No message authentication beyond TLS. A peer with the
  password could send forged state updates.
- **Availability**: Any peer can pause playback for everyone, and any peer can
  mark any other peer as Away. This is by design -- there are five of us.

For v1, this is acceptable. Future improvements could include:
- Session invite codes (short-lived tokens instead of shared password)
- Per-user key pairs for identity and message authentication

---

## Key Definitions

- **FileId**: The ed2k hash of a file's contents. All playlist operations,
  file state tracking, and content verification use this as the unique
  identifier for a file. This means a file must be hashed before it can be
  added to the playlist. Computed with the eMule/AniDB ("red") ed2k
  variant — files whose size is an exact non-zero multiple of the
  9,728,000-byte block size include a trailing empty-block hash — for
  compatibility with AniDB FILE lookups. Per-block MD4 hashes are kept
  alongside the root for transfer verification.

- **Rooms**: A rendezvous server can in theory host multiple rooms. For v1,
  there is a single implicit room per server. Multi-room support is future work.

- **ActorId**: Unique identifier for a participant in the CRDT system. Each
  client has one, and the server has a well-known server ActorId used for
  authoritative actions (EOF transitions, seek authority on file change).

---

## Future Plans

- Subtitles fused into the chat log, interleaved by timestamp, **landed**
  as the Intermixed subtitle mode (see [Subtitle Display](#subtitle-display)).
  A future refinement could interleave by the in-video timestamp rather
  than wall-clock arrival.
- Automating The List's "this week's episode is out" flag (possibly via AniDB
  episode air dates).
- Enforcing the **download-speed-vs-bitrate** half of the Downloading unpause
  rule (see [File State](#file-state)). Needs a synced eligibility signal on
  `FileAvailability::Downloading` (the downloader's measured speed vs the
  file's bitrate); only the 20% threshold is enforced today.
- Direct client-to-client connections (with or without hole punching) as a
  transfer optimization, slotted in beneath the `send(peer, message)`
  interface. Cut from v2: the relay-through-NAS path makes them unnecessary.
- Web UI using the same `ViewSpec` approach (see [ui-architecture.md](ui-architecture.md)).
