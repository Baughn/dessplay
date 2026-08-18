# DessPlay Design Document

Last updated: 2026-08-19

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
   - Choose your player (mpv or VLC; WIP placeholder -- mpv is still used)
   - Add media root directories (where your anime/shows live; terminal version only)
3. **Main screen** appears with chat pane, users list, playlist and video library

### Settings Screen

The settings screen is divided into five tabs, selected with Left/Right;
Up/Down moves between controls in the active tab. Capital `S`, the visible
`[Save]` row, or Ctrl-S where the terminal delivers it saves the complete
working copy atomically. Tabs containing a missing required value carry a
`!`, and the Save row names every missing requirement.

- **Account & connection**: Username (defaults to `$USER` on Linux/macOS,
  equivalent on Windows), server (defaults to `dessplay.brage.info`), room
  password, and Ready on startup. When Ready on startup is off the user joins
  Paused; when on they join Ready. Server and password changes apply on the
  next launch.
- **Playback & display**: Player, subtitle mode, subtitle speaker names,
  subtitle speaker colors, the limited-terminal color-overflow policy, and
  the commentary-marquee display mode.
  Player cycles between mpv and VLC and is persisted, but is explicitly
  marked **WIP -- not applied**: the client still starts mpv regardless of
  this placeholder value. Subtitle mode is off / intermixed / separate pane
  (default off; also cycled live with `F2`). Speaker names default off to
  preserve the spoiler-safe behavior; when enabled, named ASS cues render as
  `Name: dialogue` in both Intermixed and separate-pane modes. Speaker colors
  default on and affect only separate-pane lines; Intermixed is uniformly dim. **Color
  overflow** applies only when the terminal lacks true-color and more than
  ten speakers have been active in the rolling five-minute window: **Reuse
  colors** (the default) preserves the previous deterministic name hashing
  into the fixed palette, while **Disable colors** renders every speaker
  uniformly dim until the active count returns to ten or fewer.
  **Commentary marquee** chooses how this client shows the synced
  [marquee](#ai-commentary-the-marquee) line: **Marquee** (the default;
  one scrolling pass on the bottom line), **In chat** (each update
  becomes a dim local chat line instead), or **Off**. Local display
  preference, applied live; the register itself is always synced.
- **Files & transfers**: Ordered media roots, cache retention, auto-download,
  archive subdirectory policy, BitTorrent downloads, and upload limit. At
  least one media root is required;
  the topmost is marked `download target`. Roots are chosen with the directory
  picker, removed with `d`, and reordered with `J`/`K` (lowercase also works).
  A blank line after `[Add media root]` separates root management from transfer
  policy.
  Cache retention accepts `0` (delete watched downloads at session end) through
  `infinite`; auto-download defaults on. **Archive subdirectory** defaults
  on: `A` moves a cached file under a sanitized series-name subdirectory; when
  off it moves the file directly into the download root. BitTorrent defaults
  off; **disabling applies immediately** (seeding torrents are removed and
  pending imports cancelled — the mid-session escape hatch for a
  saturated uplink, see [BitTorrent Downloads](#bittorrent-downloads)), while
  enabling requires a restart when the engine was off at startup. Upload limit
  accepts human-readable byte rates
  such as `500 KiB/s` and `2 MiB/s`, or `unlimited`, and applies at restart.
- **IRC bridge**: Enabled (default on), server (default `irc.rizon.net`), TLS
  (default on, selecting port 6697 rather than 6667), and channel (default
  `#dess`). The subordinate values stay editable while disabled. A dim hint
  warns that IRC is **public**: bridged chat leaves the encrypted group. Saving
  changed IRC values reconfigures the bridge live.
- **AI commentary**: an Anthropic API token (optional, annotated
  **"Baughn only"** — nobody else is expected to set one; clearing it
  disables the feature) and a ladder-cycling comment interval (Off /
  2 min / 4 min / 10 min, default Off; dormant until a token
  exists). The token does double duty: it is also pushed to the server
  (on connect, and on any edit — clearing included) as the credential
  for the AI short-title curator, so this field is the whole lifecycle
  interface for that server-side token too (see The List). The middle preset is 4:00 rather than 5:00 so a jittered
  request reliably lands inside the Anthropic prompt cache's 5-minute
  ephemeral TTL. Both apply live. The tab's note says plainly that
  recent subtitles and a player screenshot are sent to Anthropic. See
  [AI Commentary](#ai-commentary-the-marquee).

Rows whose values do not apply immediately carry a dim lifecycle annotation
(`next restart`, `reconnects IRC`, the BitTorrent row's asymmetric
`off: immediate · on: next launch`, or the player's WIP warning). Media
roots, cache retention, auto-download, BitTorrent-off, and the AI
commentary settings reconfigure their owning actor live.

Bare `J`/`K` are used for root reordering rather than Ctrl-J/Ctrl-K, which
collide with control codes (Ctrl-J == LF) in terminals without the enhanced
keyboard protocol, consistent with the playlist pane.

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
2. The pane has three modes, cycled with `m` (`Recent Series -> All Series ->
   The List`, wrapping around), and **opens on The List by default** -- the
   spreadsheet view is the day-to-day "what are we watching" surface, so it's
   what you see first:
   - **The List** (default): see [The List](#the-list-series-tracker).
   - **Recent Series**: only franchises the user has *watched*, most
     recently watched first (then title). Unwatched series are hidden. Press
     `/` to filter by title substring (case-insensitive); the filter *removes*
     the watched-only restriction, so any series can be found. `Esc` clears the
     filter. (Filtering is gated behind `/` so the bare `m` / `s` keys stay
     live — and reliable: Ctrl-modified letters collide with control codes,
     e.g. Ctrl-M == Enter, in terminals lacking the enhanced keyboard
     protocol.)
   - **All Series**: every franchise, sorted by title or year (toggle with
     `s`). `/` filters the same way.
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
     A multi-copy episode is watched when **any** copy is — the group
     saw the episode, whichever encoding carried it — so its header
     mutes on any watched copy while the copies keep their own per-file
     marks. A `<` marker sits on the first unwatched row (skipping
     unwatched duplicates of already-watched episodes), and the browser
     opens (and a season, once selected) with the cursor already there.
   - `w` cycles the group watched flag directly (a `MarkWatched`
     request to the server, mirroring `EofReached`'s watched-flag
     write): handy for marking an episode watched without playing it to
     EOF, or undoing an accidental advance. Setting it to watched also
     runs the same List `next_ep` auto-advance the EOF path gets. On a
     copy it flips that file; on a header row it toggles the whole
     episode — any copy flagged means watched, so `w` unmarks every
     flagged copy, and marks all copies otherwise. No-op only in the
     season list.
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

**From Nyaa:**
1. With the Playlist pane focused, press `n`. This requires the
   startup-applied **BitTorrent downloads** setting; when disabled the client
   points to Settings and does not start the engine implicitly.
2. Enter a query. DessPlay searches Nyaa's Anime category (`c=1_0`) and
   inspects the first 20 RSS results' `.torrent` metadata. Only torrents with
   at least one seeder and exactly one safe payload file are listed, with
   filename, exact size, release title, and seeder count.
3. Select a result to download it in the background. The search modal closes
   and the non-blocking add-progress overlay shows download and ed2k-hashing
   stages. Reopening with `n` shows active imports; `s` starts another search
   and `d` cancels the selected import and deletes its partial data.
4. The shared playlist entry is created only after the payload finishes and
   its ed2k identity is known, inserted after the row selected when the search
   opened. Failed/cancelled imports remain local notices and never create a
   provisional shared entry.

**Sort order:** `Tab` toggles the add/map browser between
alphabetical (the default) and newest-mtime-first, both in a plain
directory listing and in search results — so files that just landed float
to the top. Newest mtime comes from the **library index** when the file
is already hashed, or a live stat for one that isn't yet; directories
always stay alphabetical (no single meaningful mtime). Not available in
the media-root directory picker (no library index there). The mapping
browser's own edit-distance-to-target ranking is what "alphabetical" means
for it -- `Tab` still switches it to newest-mtime-first. The choice is
persisted, like the All Series sort.

**Or paste it in:** pasting (terminal paste, bracketed) a single path
to a file that exists adds it — same as picking it in the browser,
anchored after the playlist's currently selected entry, whichever pane
is focused (a drag lands wherever the cursor happens to be, and there
is no use for posting a file *path* to chat). The path may arrive in
any of the shapes terminals actually produce on drag — bare,
shell-escaped (`My\ Show/ep.mkv`), quoted, or a percent-encoded
`file://` URL — each reading is tried and the first that names an
existing file wins. The file may live anywhere, including outside
every media root: an out-of-root add is registered **in place** as a
manual-mapping row (no copy into the cache), which also makes it
servable across restarts — moving the file afterwards breaks it
exactly like a moved manual mapping. Any other paste (multiple lines,
nothing that names a real file) is treated as ordinary text and lands
in the chat input instead, exactly as if typed.
While a modal is open, a paste goes to its active text editor (e.g. the
settings screen's token field) as if typed. Either way control
characters are dropped — typing can never produce one, so a copied
value's trailing newline never lands invisibly in a field (nor in a
chat message, whose bytes would sync to every peer's terminal).
Defensively, the chat display strips control characters from inbound
synced and IRC-bridged lines too, so a hostile or malformed remote
message cannot write raw escape bytes into the terminal.

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
     showing the current state. *Limitation:* the automatic Not-Watching set
     requires an AniDB **series id** to key the synced preference on; a
     missing file whose series has no id keeps blocking, and the manual
     not-watching action (4b) is the escape hatch. The "known series"
     detection itself uses the series id when present and the series name
     otherwise.
4a. You can manually map to a different file:
   - Select the red entry, press `M` to open browser
   - Browser opens to the directory most recently used for files from that
     series (the main loop supplies it from `series_map_dirs` when the
     browser is requested; unknown series open at the media roots)
   - Files are sorted by edit distance to the target filename by default;
     `Tab` switches to newest-mtime-first
4b. You can manually set yourself to "not watching" on a file that's Missing
   (e.g. a known series but you don't have this episode yet). This clears the
   "missing from known series" block
4c. By default: The file is retrieved from peers using a bittorrent-like protocol.
    Downloaded files live in the **download cache** and are evicted according to
    the retention policy; they are never automatically placed in a media root.
    An explicit **archive** action moves a cached file into the download root,
    aka. the topmost media root. By default it uses
    [Series name]/[Original filename]; the Files setting can instead
    place [Original filename] directly in the root. See
    [Download Cache](#download-cache-and-retention).

### User States

Each user has a state describing their readiness. The default value for this
can be set on the settings screen.

This state is **derived** from two independent sources:

1. **Per-series watch preference** (`Map<(UserId, ListEntryId), LwwCell<SeriesPreference>>`,
   `SeriesPreference { state: SeriesWatchState, set_by: Option<UserId> }`):
   a user's commitment to a specific series, keyed by its
   [List](#the-list-series-tracker) entry rather than by AniDB series id.
   AniDB linking is enrichment only (episode metadata, franchise grouping) --
   never a prerequisite for commitment, because a real number of series the
   group watches have no AniDB entry at all. Resolving the now-playing file
   to a `ListEntryId` (for `/watch`/`/maybe`/`/skip`, the watch-cycle key, and
   the Users-pane `n` action) auto-creates an entry on first use if none
   already claims the file -- see [Series Identity](#series-identity) for
   the resolution order. With **three** `state` values:

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
   Both write the `(UserId, ListEntryId)` preference directly; neither needs
   an AniDB link. `Ctrl-R` / "mark ready" does **not** commit: it clears a
   pause or an auto-`NotWatching` back to **Maybe**, never to Watching, so
   the block-across-absence commitment is always opt-in.

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
- **Downloading**: The user's client is actively retrieving the file from
  peers. Playback gates on the holder's **playable verdict**: the
  downloading client — the only party holding both the chunk bitmap and its
  own player position — checks whether any chunk is missing within the
  **20% window ahead of its current playback position** (position 0 before
  the file loads) and advertises the result as its availability variant
  (`Downloading` blocks; `DownloadingPlayable` doesn't), so every client
  derives the same gate with no arithmetic. The window is deliberately
  approximate — time maps to bytes proportionally; the 20% buffer exists
  precisely to absorb variable bitrate. When the window ahead of the start
  fills in, the client **loads the partial file into the player** (the
  download assembles in place at its final cache path, so completion needs
  no reload) and watches with the group; if playback catches up to a gap —
  or a seek lands past the downloaded region — the verdict flips back, the
  user gates, and the group pauses until the window refills (a seek
  re-anchors the verdict and fetch window immediately, not at the next
  position sample). The playback
  position also re-anchors the download's sequential fetch window, so the
  scheduler always fetches exactly the range whose absence would gate.
  Because the partial is sparse (unfetched regions read as zeros), the
  player can report a bogus end-of-file mid-episode; a partial's EOF
  report is therefore only believed when the last known position sits
  within a few seconds of the entry's duration — anything earlier is
  dropped, and the flipped verdict gates instead. A partial the player
  cannot open at all (an `.mp4` whose index sits in the unfetched tail)
  is not offered again until ~10% more of the file has arrived — the
  same bytes fail the same way, and repeated failures must not loop
  (the player-process analogue is the crash ladder, see
  [Player Lifecycle](#player-lifecycle)). The
  download-speed-vs-bitrate half of the original rule was **dropped**
  (2026-08-17 triage): since the anchored download policy it comes up
  rarely, and the group decides by watching how fast the download
  percentage moves.

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
| Downloading | Green | Ready & Downloading [playable from its position] |
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
- **Missing file (downloading enabled)**: File State -> Downloading; the
  placeholder shows while the first 20% window verifies, then the partial
  file loads into the player and plays with the group (see
  [File State](#file-state)). Every playlist entry the client is
  downloading also shows its percentage in the Playlist pane — downloads
  mostly happen in the background (prefetch), so progress must be visible
  without selecting the file
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
   - **< 150ms**: ignore
   - **150ms - 3s** *sustained* (a few consecutive samples -- one noisy
     sample never triggers): slew -- adjust playback speed proportionally,
     up to ±2% far out and tapering as the gap closes (mpv `speed`
     property, pitch-corrected), until the gap is under **25ms**
   - **> 3s** sustained: hard seek

   The engage (150ms) and release (25ms) thresholds are deliberately far
   apart, and mid-correction speed updates are quantized and rate-limited
   (~1/s): a sustained pitch-corrected 2% slew is inaudible, but each speed
   *transition* is a broadband click, so corrections must be few and long
   rather than frequent and brief.

   The position reference is the **seek authority's** position when a *user*
   holds authority -- **but only when that user is a valid same-file source**
   (present and advertising the now-playing file `Ready` — or
   `DownloadingPlayable`, a downloader playing the verified window; their
   partial is the real video, and when the whole group is downloading a
   fresh episode they are the only valid sources there are). A user can hold seek authority without being on the real video --
   e.g. a not-watching client whose player shows a placeholder, which still
   reports a position -- and following it would freeze the whole group on
   its bogus position. So an invalid user authority is treated exactly like
   Server authority below; it is never followed. (Symmetrically, a client
   that does not hold the real now-playing video never *takes* seek authority or
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
   one, so the group converges on the front. Without this fallback the player
   would run open-loop under Server authority: any initial offset (a
   late-starting player, a brief decode stall) would sit uncorrected for the
   whole episode.

   Eligibility -- for both the leader election and validating a user
   authority -- is restricted to peers whose position is **for this file**.
   Two gates, both required: the peer advertises now-playing as
   `FileAvailability::Ready` (or `DownloadingPlayable`), **and** their
   `PlaybackPosition` carries a `file` tag equal to now-playing. The file tag is the load-bearing one: `Ready` alone
   is *not* sufficient because it is set on **prefetch** (a peer advertises
   Ready for next week's episode long before it plays it), so right after a
   now-playing transition a peer can be Ready for the new file while its
   position register still holds the *previous* file's sample -- and
   forward-only leader election would latch the group onto that stale value
   instead of starting the new file at T=0. The tag is a clock-free identity
   check; it deliberately excludes absent users, users on a different file,
   and users watching a placeholder (file missing / still downloading / not
   watching) -- their position is stale or for another file and must never
   elect a bogus leader or be followed as authority. (The tag itself is
   trustworthy at its source because the player actor attributes positions
   to a file only after mpv's own path echo confirms that file is loaded --
   see [Events from Player](#events-from-player).)
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

Before unpausing is allowed, DessPlay verifies file contents match: the
local file's ed2k hash is compared across all Ready users, and a mismatch
blocks unpause (File State -> Missing). This prevents sync issues from
different encodes/versions. See [Content Hash](#content-hash).

### Chat

- Type in the chat input (always visible at bottom of chat pane)
- Press Enter to send
- Messages appear in the chat pane AND as OSD in the video player — a
  rolling overlay (top-left, mpv `osd-overlay`) holding the recent
  messages: each line stays a minimum of 8 seconds and expires
  individually, so a burst never erases an unread line. Your **own**
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
- **Spoiler tags**: `||spoiler||` (Discord's syntax, for familiarity)
  hides part of a message. Spoilers are a **display concern** — the raw
  `||...||` text is what syncs and archives (the CTCP-action rule: only
  the display sites decode) — and every display surface hides the run:
  the chat log renders it as deterministically scrambled letters
  (everything except whitespace and plain ASCII punctuation replaced
  class-for-class — letters and digits keep their class; CJK, emoji,
  arrows, and other symbols become letters — so nothing leaks) under
  sparse combining marks ("low-grade zalgo"); the player OSD
  substitutes the same static scramble, and the **outbound IRC bridge**
  a static scramble of its own (seeded per message, never from the
  text — see IRC Bridge, Outbound), bars dropped. In the chat pane, **clicking** the scrambled
  run plays a ~600ms re-randomization tease; a **second click within 5
  seconds** reveals the original (bars dropped) for the rest of the
  session, per client. A click after the window lapses re-teases with
  fresh letters. `/reveal` is the keyboard equivalent. The OSD and IRC
  deliberately have no reveal — IRC is public, logged, and one group
  member's primary chat surface. The sender sees their own spoilers
  scrambled too; inbound IRC and `/me` bodies parse spoilers like any
  chat text, while system, subtitle, and narrator lines never do. The
  scramble is seeded by message identity (no RNG): stable across
  repaints, identical between the chat pane and the OSD.
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
    you're absent; resolves to a [List entry](#series-identity), creating one
    automatically if the file doesn't match one yet -- no AniDB link needed)
  - `/maybe` -- set the now-playing file's series to Maybe, the opportunistic
    default (same [List entry](#series-identity) resolution as `/watch`)
  - `/skip` -- stop watching the now-playing file's series (sets your
    per-series preference to NotWatching; same resolution as `/watch`)
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
  - `/reveal` -- reveal the newest still-hidden [spoiler](#chat) on
    screen (the keyboard path for the spoiler click flow; repeat for
    earlier ones). Posts a local notice when nothing on screen is hidden.
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
  verbatim). **`||spoiler||` runs are masked at the tap** with a
  static scramble (bars dropped, CTCP framing preserved): the channel is
  public and logged, one group member reads chat *only* there, and IRC
  has no reveal affordance -- raw bars would hand the spoiler to exactly
  the people the sender hid it from (see [Chat](#chat), Spoiler tags).
  The mask is seeded from a per-process message counter, never from the
  message text -- a plaintext-derived mask would let a channel lurker
  confirm a guessed spoiler by recomputing it -- so its letters differ
  from the chat/OSD rendering of the same message (which nobody
  cross-checks).
  Long plain lines are split to fit IRC's 512-byte limit; newlines
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
| **Seek** (> 5s user scrub) | the explicit `UserSeek` carried by user seek-authority | "Baughn skipped 08:12 → 12:34" (`from` = where the debounced scrub began; `to` = where it settled) | Local (every genuine user seek, including the first in an episode; automatic load-to-zero, drift-correction, and restore seeks never create a `UserSeek`) |
| **New file** (manual select) | now-playing register change, no watched flip | "Now playing: [Frieren] - 02.mkv" | Local (the *what* persists in the playlist pane) |
| **New file** (EOF advance) | now-playing change + prior file's watched flag set | "Up next: [Frieren] - 02.mkv" | Local (ditto) |
| **Joined** | `PeerList`: a peer becomes Present | "Nero joined" | Local |
| **Connection lost** | `PeerList`: a peer becomes Lost | "Nero's connection dropped -- everyone paused" | Local |
| **Left** | `PeerList`: a peer becomes Departed (a timeout *or* a graceful `Goodbye`) | "Nero left" | Local |
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
Drift-correction slews and automatic hard seeks never create a `UserSeek`, so
they never produce a "skipped to" line. The 1500ms debounce captures the
scrub's initial and final positions and coalesces it into one authority write.
Continuous `PlaybackPosition` samples are never interpreted as seek events.

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
  is added (unwatched entries first, anchored at now-playing exactly like an
  interactive client's pre-fetch order; watched back-catalog last), making it
  the durable seed for the group: whoever adds a file can go offline once the
  seeder has it. The primary seeder is colocated with the rendezvous server, so serving
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

**Known but offline.** The above stages only cover peers the server's
in-memory registry has seen *this process lifetime* -- a user who hasn't
connected since the last server restart (or hasn't launched yet today) is
invisible to it. The server also persists a small `known_users` table
(username, last-seen millis), updated on every connect and disconnect, and
pushes it alongside `PeerList` as `known_offline: Vec<KnownUser>` (everyone
known who isn't currently Present, within a 30-day window). The Users pane
renders this as a single dim + italic list, showing "Kim (last seen 3d
ago)" for both a user who left
minutes ago and one who hasn't shown up today, unified because both are
equally valid `n` / `/skip <name>` targets (the point: rule on
someone's commitment without waiting for them to reconnect). A committed
(Watching) absent user is excluded from this list and shown instead as the
red "committed, away" blocker row, exactly as before.

**Known-offline users gate too (for a week).** Playback gating quantifies
over peer entries, and the server's registry spans only its own process
lifetime -- so a server restart would silently waive every absent user's
commitment. Clients therefore merge `known_offline` into the peer list before any
derivation (`derive::merge_known_offline`): each known-offline user seen
within the last **seven days** is synthesized as a Departed interactive
entry, so a committed user blocks -- and reads as the red "committed,
away" row -- exactly as if they had departed this session; a Maybe or
NotWatching user's synthesized entry changes nothing (absent Maybe never
blocks). Real peer entries are never shadowed. Past seven days the
synthesis ages out: the horizon matches the commitment's own "wait for me
even if I've been gone a week" phrasing, and bounds how long a stale
username (e.g. after a naming-convention change -- commitments are keyed
by username, and identity aliasing was deliberately rejected as a fuzzy
heuristic in the gating path) can keep blocking. A stale blocker is
dismissible at any time via `/skip <name>`, marking them Away, or a
per-file `/ack`. Seeders never appear (only interactive connections are
recorded in `known_users`).

---

## The List (Series Tracker)

The group's shared tracking spreadsheet, ported into the app: an explicit,
permanent record of what the group plans to watch, is watching, and has
finished. This is **separate from the playlist** (which holds concrete files
for a session); List entries are series-level. `anidb_series_id` links an
entry to AniDB for **enrichment only** -- episode metadata, franchise
grouping, the AniDB search modal -- and is never a prerequisite for
commitment or gating: a real number of series the group watches (obscure
OVAs, doujin work, non-Japanese content, very new simulcasts) simply have no
AniDB entry, and the design must not depend on them getting one. Every
series anyone commits to via `/watch`, the watch-cycle key, or a List
entry's `watchers` set has a List entry, linked or not -- see
[Series Identity](#series-identity).

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
    anidb_series_id: Option<AniDbSeriesId>,  // linked manually after import; enrichment only
    local_aliases: BTreeSet<String>,  // confirmed series-name aliases (unlinked matching)
    manual_files: BTreeSet<Ed2kHash>, // explicit file overrides, for names aliases can't catch
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
OVA names, and guesses. See [Advancing next_ep](#advancing-next_ep) for how
and when it auto-advances, linked or not. Otherwise `available` is
maintained by hand; automating it (e.g. via AniDB episode air dates) is
future work.

The List is never pruned: it is a few hundred rows of text, and the history
is the point.

### Series Identity

Two mechanisms already turn a file into "a series," and neither is a safe
foundation for group **commitment** (whether the group waits for someone
across absence):

- AniDB's relations graph, for files with a series id -- structural
  (sequel/prequel/etc.), stable, but only exists for series AniDB knows.
- A per-file **derived name** (the AniDB-miss fallback's `series_hint`, or
  the bare filename otherwise -- see
  [Parsing files](#parsing-files-to-seriesseasonepisode)), used today for
  franchise-browsing's fallback grouping and personal known-series
  detection. This name is **not** stable enough to hang commitment on: it's
  computed per file, from that file's own directory context, and the group
  does not reliably keep every episode of an untracked show in one
  dedicated directory. Two files of the same show, one hinted from
  `Anime/ShowName/` and one sitting loose elsewhere, derive different
  names -- silently splitting one show into two "series" for the one
  question that most needs a single stable answer.

So List entries carry their own identity data, used only for unlinked
entries (a linked entry's AniDB series id is authoritative and skips all of
this):

- `local_aliases`: confirmed series-name strings that resolve to this
  entry -- seeded with the derived name of whichever file first created the
  entry, and grown by hand (same edit modal) whenever a differently-hinted
  file for the same show shows up.
- `manual_files`: explicit file hashes attached directly to the entry, for
  outliers whose name doesn't parse into any alias at all.

**Resolution order**, used by `/watch`/`/maybe`/`/skip`, the watch-cycle
key, the Users-pane `n` action, and `watchers`-set wiring, given the
now-playing (or selected) file:

1. The file has an AniDB series id, and some List entry is linked to it:
   that entry.
2. The file's hash is in some entry's `manual_files`: that entry.
3. The file's derived name matches some entry's `name` or `local_aliases`:
   that entry.
4. No entry claims the file: auto-create one -- linked, with name/nero_name
   seeded from `AniDbMetadata`, if the file has a series id; otherwise
   unlinked, with `name` and the sole `local_aliases` entry seeded from the
   derived name, so a later file with the same derived name matches without
   further setup.

This is deliberately **stricter** than the existing franchise-browsing and
known-series heuristics, which stay soft, best-effort, and unchanged (an
"accepted" cosmetic edge case, per [File Matching](#file-matching)) -- a
mis-grouped franchise row is a browsing annoyance, but a mis-resolved List
entry silently un-commits someone from a show they're actively watching.

### Advancing next_ep

Two distinct problems hide under "auto-advance," with very different
certainty:

1. **Bumping the counter.** When the group finishes a file belonging to a
   linked series, the server increments `next_ep` from that file's own
   `AniDbMetadata.episode` (authoritative) and resets `available` to false.
   For an **unlinked** entry the same bump happens from the *just-finished*
   file's own filename-parsed episode number, when one parses cleanly --
   there's no real ambiguity here, since it's a fact about a file already
   confirmed watched, not a guess about one that hasn't aired yet. A file
   whose name yields no parseable number leaves `next_ep` for a manual bump,
   exactly like any other free-text entry ("movie 5?").
2. **Resolving the counter to a file.** Finding *which* library file is
   episode `next_ep + 1` is the genuinely uncertain step for an unlinked
   series -- there's no AniDB episode identity to match against, only
   heuristics (filename-parsed episode number, edit distance to the
   expected label, mtime recency, `local_aliases`/`manual_files`
   membership). Rather than guess silently, jumping to `next_ep` on an
   unlinked entry reuses the Episode Browser's existing multi-file
   disambiguation UI ("several files claim the same episode number ...
   expand into a lightweight tree," see [Adding Files to the
   Playlist](#adding-files-to-the-playlist)) -- generalized from "several
   files, one confirmed identity" to "several *candidate* files, ranked by
   score, no confirmed identity." Picking one runs the ordinary
   add-to-playlist flow; nothing is queued until a human picks. This is
   deliberately **not** a new kind of synced Playlist entry -- no
   `Map<Ed2kHash, ...>` schema change -- it lives entirely in the
   Series/List pane and the episode browser, exactly like choosing which
   copy of a linked episode to play today.

### UI Integration

The Series pane gains **The List** as a third mode (alongside Recent Series /
All Series) -- and is the pane's default mode, see [Adding Files to the
Playlist](#adding-files-to-the-playlist). Grouping answers "what are we
watching, and whose": the Watching tier (CurrentSeason + Active) renders as
one **"Watching — ⟨user⟩" group per committed user** -- every peer or
known-offline user whose `series_preference` for the entry is Watching, the
local user's group first and the rest alphabetical, an entry appearing in
every applicable group -- with a residual shared **Watching** group for
Watching-tier entries no rendered group claims (no committed watcher, or
one the client can't name), so nothing vanishes. Below that the shared
status groups: ShortList, Planned, Waiting, Hiatus, with Finished/Dropped
collapsed at the bottom. Pressing `Enter` on an
Active/CurrentSeason entry jumps toward `next_ep`: for a linked entry, into
the episode browser with the cursor on that episode if anyone has it; for an
unlinked entry, into the candidate-ranked disambiguation view described
above. Either way queueing tonight's episode is a couple of keypresses.

Within each group, `s` toggles the sort (persisted across sessions, like
All Series' sort): **Recency** (the default) floats watchable entries --
the weekly `available` flag set, or a **held** unwatched file: some client
advertises a copy (the availability map) that neither the group watched
flag nor personal watch history records as seen (the episode browser's
muting rule), resolving to the entry by the Series Identity order. A bare
metadata row is not enough -- metadata persists forever, files don't --
and a duplicate encoding of an episode the group watched through *any*
other copy doesn't count either (the same any-copy rule, by AniDB
episode identity).
Watchable entries sort above those with nothing to watch, most recently
watched first within each partition (from local watch history, the same
source as Recent Series); **Alphabetical** is plain name order. Entries
with nothing to watch render dim in either sort -- the dim set is exactly
Recency's bottom partition.

Entries display name, nero_name, next_ep, and **live commitment initials**:
the users whose `series_preference` is Watching, not the import-time
`watchers` seed (which keeps its one-shot preference-seeding role and is
never displayed). A **linked** entry whose `SeriesRelations` carries a
curated community short title renders it *instead of* the official
name, and alphabetizes under it — "GochiUsa", not "Gochuumon wa Usagi
Desu ka??" (user decision 2026-08-17: save the space; the full name
still lives in the edit modal and the episode browser) — but only when
the entry's name still equals the official title (the auto-seeded
case): a name a human typed or edited always wins (user decision
2026-08-18), which is also the fix-it path for a bad curated pick.

The short titles are **AI-curated**, not read raw from the titles dump:
the dump's kind-3 rows are lowercase search tags ("gochiusa s2", "s;g",
"HnNKn") and only a quarter of series have one, so the server's worker
sends each series' full title rows to an Anthropic model once
(trusting the answer as returned — the human-name precedence above is
the backstop) and caches the answer forever in its SQLite; the
reconcile pass then keeps the replicated `short_titles` in step with
that cache. The API token is **client-provisioned**: it lives in one
client's settings (the same `anthropic_token` the commentary engine
uses), is pushed to the server over the encrypted control connection on
connect and on any settings edit that changes it
(`SetAnthropicToken`, protocol v12), and the server persists it — so
the settings screen is also the interface for rotating or removing the
server-side credential. No token, no curation; nothing else degrades.

`nero_name` is appended dim-quoted as always, and gets its own
fast entry path: `n` on an entry opens a minimal single-field editor
(Enter saves — trimmed, empty clears — Esc cancels), the group's
renaming culture in two keystrokes; the full edit modal remains for
everything else. A linked entry whose free-text `next_ep` parses as a
plain episode number renders it as **`SnEnn`** -- the season ordinal
counted along the replicated prequel chain (best effort: cycle-guarded,
counting the visible prefix when the graph hasn't filled in) -- with an
"out" marker from `available`; anything else ("S3-05", "Sisters",
"movie 5?", unlinked entries) renders verbatim. Editing fields and adding entries happens
in a small edit modal (also where `local_aliases`/`manual_files` are edited
for an unlinked entry); linking an unlinked entry (`l`) opens the AniDB
search modal: it pre-searches for the entry's name (informal names like
"GochiUsa" resolve through the titles dump's synonyms), the user picks
from the ranked candidates and confirms. Enter on fresh results links;
editing the query re-arms search. Linking does not require or touch
`local_aliases`/`manual_files` -- an entry can be linked and still carry
them, in case a stray file's derived name never matched the AniDB-known one.

The `watchers` set wires into the per-series watch preference, and is the
declarative route to **commitment**: users *in* the watchers set get
`SeriesWatchState::Watching` (committed -- the group waits for them even
when absent) and users *not* in it get `SeriesWatchState::NotWatching` (so
they never gate playback on shows they skip) -- linked or not, since
commitment keys on the entry's `ListEntryId`, never its AniDB link. Series
with no List opinion stay at the **Maybe** default. Two guards: an *empty*
watchers set means "unrecorded", not "nobody", and never writes
preferences; and an existing preference (a manual choice) is never
overridden.

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
  linked lazily from the UI. `local_aliases`/`manual_files` are likewise
  empty on import -- an imported entry only starts claiming files once
  something (a link, or a `/watch` on a matching file) resolves to it.
- **Re-import preserves app-owned fields.** A re-run (the supported way to
  refresh statuses/next_ep from the sheet) matches existing entries by
  name and carries over what the app owns and the CSV cannot express: the
  AniDB link, `local_aliases`, `manual_files`, and the
  AniDB-search-came-up-empty flag. Only sheet-owned columns are
  overwritten.

---

## TUI Layout

### UI Principles

**One color scheme when true-color is available.** During production terminal
setup DessPlay asks crossterm for the terminal's advertised color count once,
supplemented by the standard `COLORTERM` and `*-direct` `TERM` hints. A
true-color terminal gets an explicit app-wide dark theme: the complete
alternate-screen buffer uses a known dark background and mapped RGB semantic
foregrounds, including panes, modals, and passive overlays. This makes text
contrast deterministic instead of depending on the user's terminal theme.
Dim semantic text is materialized as an explicit muted RGB foreground in this
final pass rather than left as SGR 2, whose treatment alongside explicit RGB
colors varies between terminal emulators. Other text modifiers are preserved.
A terminal without true-color retains its own foreground/background theme and
uses DessPlay's finite ten-color application palette where identity colors are
needed; its dim text continues to use the terminal's native attribute. The
capability is injected into the synchronous `Ui`, so tests do not depend on
process-global terminal state.

**No silent long-running work.** Any operation that can take more than
a moment (hashing a file for the playlist, scanning media roots,
downloading from peers, archiving) must show visible progress in the
UI while it runs — a user who sees nothing happen assumes nothing is
happening, and retries. Playlist-add hashing shows a centered progress
overlay (one bar per in-flight file); it is visually modal but captures
no input, so chat keeps working underneath. Transfers reuse the same
pattern.

The **server link** is part of this: whenever the client is not
connected, the status bar's play-state slot shows the link instead —
"⚡ connecting to server (attempt N)…" while dialing (a dead handshake
can take the full per-address timeout ladder; see
[network-design.md](network-design.md#connection-types), Dialing) and
"⚡ connection lost — retrying…" after a mid-session drop. Stale gating
text ("⏸ paused") while silently failing to connect reads as a hang.

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
| [==>  ] 12:34/24:00  commentary   ▲1.2M ▼340K sync ok|
+----------------------------------+------------------+
|  Player Status: waiting on baughn (paused)          |
|  Now Playing: [Frieren] Sousou no Frieren - 01.mkv  |
+-----------------------------------------------------+
| Tab Next pane | Enter Send | Esc Clear | Ctrl-C Quit |
+-----------------------------------------------------+
```

**Proportions:**
- Bottom: Player status (3 lines) then keybinding bar (1 line)
- Above that, the main area's last row is one **terminal-wide bottom
  line**: the progress bar + time at the left (its own row, kept off the
  bottom status bar so the variable-width "waiting on ..." blocker text
  never shoves it sideways; the same placement in every subtitle mode), the
  [Connection Health Line](#connection-health-line)'s metrics
  **right-aligned** at the terminal edge, and the middle space carrying
  the suggestion slot, centered with a couple of spaces of margin.
  Reserving the row before the column split keeps the playlist's bottom
  border level with the chat input's.
- Left 50%: Chat (with input line at bottom)
- Right 50%, top: Series (three modes: Recent Series / All Series / The List)
- Right 50%, middle: Users
- Right 50%, bottom: Playlist

When any selectable list is taller than its pane or modal, its viewport keeps
the cursor as close to the vertical center as the list edges permit. Series
and Users retain that cursor-centered context while unfocused; Playlist
centers on the now-playing entry while another pane is focused. The chat log
is deliberately separate: it keeps its history/newest-first scrolling policy
rather than following a selection cursor.

### Connection Health Line

The **right-aligned end of the terminal-wide bottom line** is a
borderless, passive status field showing connection quality and sync
health at a glance — a saturated uplink can let BitTorrent drown CRDT
sync while the QUIC connection stays nominally "connected", and this
row is what makes that visible. (The same line's left end is the
progress bar; the middle is the suggestion slot below.)

While connected it renders compact metric fragments, joined with dim
separators: `▲1.2M ▼340K · rtt 89ms · sync ok` —

- **▲/▼**: upload/download bytes per second, the QUIC plane (control,
  datagrams, relayed transfer) **plus** the torrent engine's live
  speeds, so the culprit of a saturated uplink is visible here even
  though it never crosses the server connection.
- **rtt**: the median time-sync probe round trip (the probes are
  datagrams, so this reflects real path latency — bufferbloat shows up
  as seconds); before any probe is answered, QUIC's own path estimate.
- **sync**: seconds since *anything* arrived from the server. The
  server broadcasts a `StateHash` every 30s unconditionally, so this is
  a zero-false-positive stalled-sync detector: a large value on a live
  connection means sync is dead even though QUIC is not. Displayed as a
  static **`sync ok`** while the age is unremarkable — a counting
  number draws the eye, and what counts as remarkable depends on how
  chatty the wire should be. During group playback (another interactive
  peer present) peers' position datagrams arrive continuously, so the
  age is shown from 5s of silence; alone or idle the only incoming
  traffic is two interleaved 30s heartbeats (the age legitimately
  sawtooths toward ~30s), so it is shown only past the 40s warning
  threshold, where it colors anyway. Display only — the health
  classification below is unaffected.
- **N probes lost**: shown only when consecutive steady-state probes go
  unanswered.

The row is **dim by default**; only the *offending* field takes a
warning color. Classification (thresholds in `ui/props.rs`,
`classify_health`): **Degraded** (yellow) at rtt ≥ 1.5s, silence past
one missed StateHash interval (40s), or 2 lost probes; **Stalled**
(red) past 2.5 missed intervals (75s), or 3 lost probes with 45s+
silence. The displayed level is hysteresis-filtered — trouble shows
immediately, calm must hold ~5s (stepping down through intermediate
levels) — so a single quiet sample never flickers red back to dim.
While not connected the row shows a short link notice
(`link: connecting…` / `link: down — retrying`); the bottom status
bar keeps its existing, fuller `⚡` story.

The middle of the row — between the progress bar and the metrics,
centered with at least two spaces of margin toward each neighbour — is
the **suggestion slot**, fed by the session-layer **advisor**:
rule-based advice keyed to the health state — "high latency — disable
BitTorrent (F3, applies immediately)" when the link degrades with an
active torrent (and the toggle really does apply immediately — see
[BitTorrent Downloads](#bittorrent-downloads)), "sync stalled — server
silent Ns", and a divergence notice. Suggestions carry a severity
(dim / yellow / red), only re-render when they change, and a cleared
condition holds the slot ~30s against threshold flicker — but a full
disconnect clears the slot at once (the `link:` notice supersedes it;
a condition persisting across the reconnect re-emits). When the row
is tight the health metrics keep their full width (they are the row's
reason to exist), the progress bar truncates next, and the suggestion
takes whatever middle space remains — dropped entirely rather than
rendering a lone ellipsis. The slot's claim on the bar is
**text-width-capped** — it reserves its occupant's text plus the
2-space margins, the marquee included (a window as wide as the line
shows all of it mid-pass), so the bar never shrinks further than the
middle actually needs; an empty middle reserves nothing. The slot is
also where the
[AI commentary marquee](#ai-commentary-the-marquee) scrolls; slot
precedence is **warning/critical suggestion > live marquee > info
suggestion > blank** — a health warning is the row's job, so the
commentary yields to it.

The row is dead to the mouse (it is outside every pane rect).

### AI Commentary (the marquee)

Just for fun, and explicitly a **single-user gimmick** (the settings tab
says "Baughn only"): on the configured interval — jittered ±15 s per
comment so it isn't metronomic, and only while connected, playing, and
holding the now-playing file — the client with an Anthropic token asks
**claude-opus-4-6** (adaptive thinking at low effort, hardcoded — the
forward-compatible request shape, never the deprecated fixed
thinking-token budget) to react to the
episode *in character*, and the reply scrolls across the bottom line's
middle slot on **every** client.

- **The commentator.** The voice is a persistent character from the
  show's cast: a first, spoiler-bounded call asks for "major characters
  who have appeared up to and including this episode only", and the code
  picks one at random. The pick persists across ticks (and across API
  failures) and is **re-rolled with 5% probability per tick** — a quietly
  changing persona is funnier than a fresh voice every time. It is
  deliberately **not** reset on a series change: the voice follows the
  group to the next show — Hinamori Amu commenting on Grave of the
  Fireflies is an accepted (welcomed) outcome — until the dice or a
  client restart retire it. The character card stays pinned to the
  commentator's *home* series, so a carried-over voice knows it is
  watching someone else's show.
- **The thread.** Each commentator is a real multi-turn conversation,
  not a stateless call: the character card and rules (the spoiler bound
  — "you know nothing beyond the episode currently being watched" — and
  the output shape, one IRC-style line `<Amu> Whaaaat?`) live in the
  system prompt, and every tick appends a user turn carrying only the
  subtitle lines that arrived **since the last comment** (the advisor
  ring's per-line sequence numbers are the cursor — consecutive turns
  never overlap, and a failed call doesn't advance the cursor),
  **speaker-attributed** — a cue with an ASS Name field goes out as
  `Name: line`, the same field the separate subtitle pane colors by,
  since a model that can't watch the video needs the dialogue
  attributed (a nameless cue stays bare) — plus an
  mpv screenshot when one can be captured in time
  (`screenshot-to-file`, raw frame, no OSD/subs; best-effort — its
  absence never blocks the tick). The model's replies ride along as
  assistant turns, so the commentator remembers what it already said.
  An episode — or series — change stays in-thread: the next turn opens
  with a "Now playing" header. Episode identity is keyed by the
  now-playing **file** (alongside series name and episode label) —
  AniDB-unknown files all share one hint-derived series name and no
  episode number, so without the file in the key an unlinked series'
  episode changes would never re-header (or reset the comment seed
  below). A commentator change (the 5% re-roll)
  cuts the thread; the fresh commentator's first turn is seeded with
  the **text** of the current episode's earlier comments — never the
  images or subtitles behind them — so the voice changes without the
  conversation restarting from nothing. The 5% re-roll keeps threads
  young *in expectation*, but its tail is geometric, so frames are
  capped separately: only the most recent one or two turns keep their
  screenshot bytes (the text history is never trimmed, so the
  conversation — and the cached prefix up to the trim point — stays
  intact).
- **Caching.** When the interval (jitter included) fits inside the
  Anthropic prompt cache's 5-minute ephemeral TTL — the 2 min and
  4 min presets; the 4 min preset exists precisely to duck under
  the TTL — each request marks the final text block with an ephemeral
  `cache_control` breakpoint, so the append-only thread re-bills at
  cache-read rates instead of full price. At 10 min the cache would be
  cold anyway and the write surcharge is skipped. Per-call token usage
  (input, output, cache read/write) is logged at info.
- Replies are normalized (newlines flattened, missing `<Name>` prefix
  repaired, hard-capped ~220 chars).
- **Distribution.** The line is written to the synced, deliberately
  generic **marquee register** (`LwwCell<Option<MarqueeMessage>>`,
  cleared at compaction like other ephemeral session state); every
  client — the author included, via the ordinary sync echo — plays the
  same marquee. How it is *shown* is a local choice: the
  **commentary-marquee** setting (Playback & display tab) can instead
  fold each update into the chat log as a dim local line (still one
  line per LWW stamp, and a pre-startup stamp is still never
  replayed), or hide updates entirely — either way the stamp is
  adopted, so switching back to the marquee never replays an old
  message. One update = **one pass**: the text enters entirely
  off-screen right (the entry delay gives people time to notice motion
  and glance down before the sentence starts leaving), scrolls left at
  ~15 cells/s, exits entirely off-screen left, and the slot reverts to
  the advisor suggestion. A pass is keyed by the register's LWW stamp —
  a rewrite replays even with identical text; the same stamp never
  restarts. A stamp from **before this session's first snapshot** never
  plays at all: the register persists in synced state until compaction,
  so a freshly started client would otherwise replay last night's final
  comment on launch — it is adopted as already-played instead. While a
  pass animates the UI thread ticks at ~100ms instead of its lazy 1s
  (idle cost unchanged — a tick only repaints when something moved).
- **Failure policy.** Every failure — HTTP error, refusal, empty cast,
  malformed reply — is a log line and a skipped tick; never a chat line,
  never user-visible noise. An in-flight call never stacks with the next
  tick, and disabling mid-flight discards the late result. The feature
  narrates itself in the log at **info** — whether it is enabled (at
  startup and on every settings change, with the reason when it is not),
  each outgoing request, the commentator it picked, each call's token
  usage (input, output, cache read/write — "is the cache hitting?" is
  one grep away), and the comment that came back — since a gimmick that
  speaks once every few minutes is otherwise indistinguishable from a
  broken token. Skipped ticks (paused, file not held, still in flight)
  log their reason at debug.

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

**Mouse support:** a left-click focuses the pane under the pointer and,
in the list panes, simultaneously selects the clicked row (the seeders
line and other non-selectable rows are ignored). The wheel scrolls the
pane under the pointer only when that pane is **already focused** (the
chat scrolls its log, list panes move their selection like Up/Down);
over an unfocused pane it is ignored — touchpads emit wheel events by
accident, so a graze must neither scroll invisibly nor steal focus.
Clicking never activates a row (no double-click Enter); the one
click-driven action is the chat [spoiler](#chat) reveal, whose key
equivalent is `/reveal`. Mouse events are ignored while a modal is
open. Keyboard-only terminals lose nothing — every mouse action has a
key equivalent, with one deliberate exception: chat text selection
(below) is mouse-native, and uncommon enough to need no keyboard path.

**Chat text selection:** click-and-drag over the chat log selects text
for copying (the terminal's own selection needs Shift once mouse
capture is on, and knows nothing of panes). Releasing the button
**copies immediately** to the system clipboard (`arboard`; local
machine only — over SSH the copy quietly degrades). No copy key is
involved: the terminal owns Cmd-C, and Ctrl-C stays Quit. A drag
within one message selects a char range and copies it verbatim,
exactly as displayed (a hidden spoiler copies as its scramble —
WYSIWYG, no leak). A drag that crosses a message boundary snaps to
whole lines and copies them in the irccloud log format —
`HH:MM:SS <nick> body` (`* nick body` for actions; day separators are
render furniture, skipped) — and a selection is always one of those
two shapes, never a mix. The reverse-video highlight is held for 5
seconds after release; while held, Shift-Up/Down extend the selection
one whole line at a time (a partial selection first widens to its
whole line, gdocs-style), re-copying on each step. Any other key or
click — or the timeout — dismisses the highlight; the clipboard keeps
the last copy. A motionless click never touches the clipboard.

### Keyboard Shortcuts

| Key | Context | Action |
|-----|---------|--------|
| `Ctrl-C` | Any | Quit |
| `Shift-Up` / `Shift-Down` | Held chat selection | Extend the selection one whole line up/down (re-copies) |
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
| `Ctrl-T` | Chat | Transpose the two characters around the cursor (readline-style) |
| `Left` / `Right` | Chat | Move cursor |
| `Ctrl-Left` / `Ctrl-Right` (or `Alt-`) | Chat | Move cursor by word |
| `Home` / `End` (or `Ctrl-A` / `Ctrl-E`) | Chat | Move cursor to start/end of line |
| `m` | Series | Cycle mode: Recent Series -> All Series -> The List |
| `s` | Series (All mode) | Toggle sort: by title <-> by year |
| `s` | Series (List mode) | Toggle sort: recency <-> alphabetical |
| `/` | Series (Recent / All) | Start filtering franchises by title (removes Recent's watched-only default) |
| _printable_ | Series (filtering) | Add to the filter text |
| `Backspace` | Series (filtering) | Delete a filter character; on an empty filter, exit filtering |
| `Esc` | Series (Recent / All) | Clear the filter (and exit filtering) |
| `PgUp` / `PgDn` | Series | Move the selection by a page |
| `Enter` | Series | Browse franchise (episode browser or file browser) |
| `Enter` | Series (List mode) | Jump to next episode / open entry |
| `e` | Series (List mode) | Edit entry (modal) |
| `n` | Series (List mode) | Edit the entry's `nero_name` (minimal single-field editor; empty clears) |
| `l` | Series (List mode) | Link entry to AniDB (search modal) |
| `Enter` | Episode Browser | Select season (cursor on its first unwatched row) / choose an episode or copy; no-op on a header row |
| `w` | Episode Browser | Cycle the group watched flag: the selected file, or every copy of the episode on a header row; no-op in the season list |
| `PgUp` / `PgDn` | Episode Browser | Move the selection by a page |
| `Esc` / `Backspace` | Episode Browser | Go back (episodes -> seasons -> close) |
| `Enter` | File Browser | Open directory / choose file (add or map) |
| `Backspace` | File Browser | Up one level (from the roots listing, close); while searching, delete a search character |
| `Esc` | File Browser | Cancel; while searching, clear the search |
| _printable_ | File Browser (add / map) | Type-to-search the library recursively (root-relative paths, directories first); not in the directory picker |
| `Tab` | File Browser (add / map) | Toggle sort: alphabetical <-> newest mtime first (persisted); not in the directory picker |
| `PgUp` / `PgDn` | File Browser | Move the selection by a page |
| `s` | File Browser (directory picker) | Select the current directory |
| `a` | Users | Mark selected user as Away (or clear an Away you set) |
| `n` | Users | Mark selected user NotWatching for the now-playing series (works on a known-offline row too) |
| `Enter` | Playlist | Play selected entry (or open file browser on [Add New]) |
| `a` | Playlist | Add file (insert after selected entry) |
| `n` | Playlist | Search Nyaa for a single-file anime torrent; reopen to manage/cancel active imports |
| Paste | Playlist | Add a pasted existing-file path (insert after selected entry); any other paste goes to the chat input instead |
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
   metadata, and fetching the relations graph for franchise grouping —
   plus the AI short-title curator (see The List), whose Anthropic token
   is provisioned over the wire by whichever client holds one
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
| Seek Authority | `LwwCell<SeekAuthority>` (`Server \| User(UserSeek { user, file, event_at, from_millis, to_millis })`) | Standalone register; last seeker is position authority, with the explicit user action that granted it |
| Playback intent | `LwwCell<PlaybackIntent>` (`Playing \| Paused`) | Standalone register; users write on play/pause, server forces Paused on lost/graceful-quit/EOF-advance (not on the timeout-ladder Departed promotion -- already paused at Lost) |
| Series preference | `Map<(UserId, ListEntryId), LwwCell<SeriesPreference>>` | Compound key, keyed on the List entry (not AniDB id -- see [Series Identity](#series-identity)); `SeriesPreference { state: Watching \| NotWatching \| Maybe, set_by: Option<UserId> }`, absent entry = Maybe; any user may write |
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
| Marquee | `LwwCell<Option<MarqueeMessage>>` | Written by the commentary-running client; every client scrolls it on update; cleared at compaction |

All registers are `LwwCell<V>` — DessPlay's own max-merge LWW register
(`crdts::MVReg` proved non-convergent inside `Map`; see sync-state.md).
`ActorId` type parameters omitted from the table for brevity -- all CRDTs
use `ActorId` as the actor type. See [sync-state.md](sync-state.md) for
the full `Lww<V>` design.

Whether the video actually plays is **derived**: it plays iff playback
intent is Playing and no interactive user blocks -- see
[Playback Rules](#playback-rules) for who blocks and why the intent
register exists.

### Chat Protocol

Chat is a `crdts::GList` (grow-only list) of (sender, text, timestamp)
messages. New messages are
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
table (path -> ed2k root + per-block hashes, keyed by `(mtime, size)`).

- **At startup** the client walks every media root, `stat`s each file, and
  hashes anything new or changed (a path missing from `hash_cache`, or whose
  `(mtime, size)` disagrees). Unchanged files are a cache hit -- no re-read.
- **Periodically** the client re-walks the roots and re-hashes only changed
  files. Interactive clients rescan about once a minute; a seeder, whose store
  is large and stable, rescans once a day.
- **Hashing yields to transfers.** Scan hashing is bulk disk work
  with no deadline, while transfers are latency-sensitive (a source that
  serves nothing for 30s is snubbed) — so while transfer traffic (serving
  or downloading) is active, scan hashing defers, resuming ~10s after the
  traffic goes quiet. The walk itself (stat-only) still runs. One
  exemption: a walked file whose name matches an unmet playlist entry is
  resolved (and so hashed) immediately, even during transfers — otherwise
  an active download would defer the discovery of the very file that makes
  it redundant (see [Download Cache](#download-cache-and-retention), "a
  local copy trumps the download").
- **The walk also reconciles disappearance per media root.** If at least one
  previously indexed file in a root still exists, the root is online and
  missing sibling rows are genuine moves/deletions, so they are removed
  immediately. If *none* of that root's recorded files exists, the client
  assumes removable storage was disconnected: it retains the hashes
  indefinitely but marks the root vanished. Vanished rows are hidden from
  library browsing/search and lookup announcements and are not advertised as
  locally available. When any recorded file returns, matching `(mtime, size)`
  rows reactivate without re-hashing and genuinely absent siblings are pruned.
  Rows outside media roots (the download cache) remain governed by their own
  startup reconciliation.
- Removing a root from the effective runtime root list hides it immediately
  and starts a **seven-day grace period**. Re-adding the identical path within
  that window preserves its hashes; after seven days the root record and its
  index rows are deleted. A configured-but-vanished root never expires.
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

A playlist entry is matched to a local file **by content hash**. The ed2k
root is the file's identity; the filename recorded in the entry is display
metadata and a scan-priority hint, never the match key. Any local file with
the right hash — whatever its name, wherever it sits — is a viable copy.

When a playlist item is added (or an entry re-resolves):

1. **Look the entry's hash up in the library index** (`hash_cache`). Every
   live row — media roots and the download cache alike — whose ed2k root
   equals the entry's hash is a match: store its path, the entry is Ready.
   Rows under a vanished root don't count (see
   [Media Library Scanning](#media-library-scanning)); a row whose
   `(mtime, size)` no longer agrees with the disk is stale, not evidence.
2. **If the index holds no match**, search the media roots for a file with
   the entry's exact basename and hash it now. This is the
   freshly-arrived-file fast path — a copy dropped in moments ago that no
   scan has reached — and the *only* on-demand hashing resolution ever
   does. A name match with the wrong hash is a different encode: it goes to
   the mismatch re-check watcher (see [Content Hash](#content-hash)), not
   to Ready.
3. **Otherwise: Missing** (red in UI), and the entry's hash joins the
   **wanted set**. Whenever a hash subsequently enters the live index — a
   scan hashes a new or changed file, a vanished root's rows reactivate, a
   download completes — it is checked against the wanted set and the entry
   adopts the copy on the spot, by hash, whatever the file is named.

Resolution never walks the disk hashing candidates: the index is expected
to already exist (the scanner builds and maintains it ahead of demand),
and a copy the index hasn't absorbed yet is picked up by step 3 when the
scan reaches it. The basename search in step 2 exists only to close the
gap between "the file just appeared" and "the next scan pass" — it is an
optimization, and missing it is never load-bearing.

*Implementation note:* this requires the index to answer by-hash lookups
(`hash_cache` is keyed by path; a reverse map or SQL index on the root
serves step 1 and the wanted-set check in step 3).

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
re-download; a surviving row is re-registered as a servable copy. The
reconciliation is bidirectional: it also **sweeps orphans** — hash-named
cache files with *no* `cache_entries` row. An orphan is either a completed
download whose bookkeeping was lost (a DB reset leaves the files but not the
rows) or an abandoned peer-download partial (`download_path` is the final
`<cache>/<hash>` path, so an interrupted download leaves one). Because
eviction only iterates `cache_entries`, an orphan is invisible to it and
would leak forever. Orphans **older than a week by mtime** are deleted at
startup — matching the "in-flight downloads don't survive restarts"
contract — while anything more recent is left alone (it may still be in
flight or wanted). Resolution
then finds a cached download like any other local copy — by hash, through
the index (see [File Matching](#file-matching)); the cache being
hash-named additionally lets `<cache>/<hash>` be checked directly, with no
index row needed. Two **runtime guards** cover deletions that
happen mid-session rather than between runs: a player load failure
(file gone under us) and a serve-time absence (a peer asks for a file we no
longer hold) both drop the local copy, prune its bookkeeping, and flip the
file to Missing so it re-resolves.

**Retention** (`cache_retention`, per client): a cached file becomes
*evictable* once it is no longer needed — either it has been watched (85%
rule, or it sits behind the group's progress via the watched flag) **or it
is no longer referenced by the playlist at all** (an abandoned download must
not pin cache space just because nobody happened to watch it). An evictable
file is deleted `cache_retention` after its last access. Special values:

- `0`: deleted at the next eviction pass after watching -- the
  "small laptop" setting; nothing accumulates
- `infinite`: never deleted -- the NAS/seeder setting

Eviction passes run at startup and on EOF-advance. The now-playing file and
queued unwatched playlist entries are never evicted, regardless of retention.

**Archive**: an explicit action (`A` in the playlist pane) that moves a cached
file under the download root (the topmost media root). The default **Archive
subdirectory** setting produces `[Series name]/[Original filename]`; when
disabled, the destination is `[Original filename]` directly under the root.
This is the deliberate "keep this in the library" decision; retention is the
default "it was just for the watch party" path. There is no `Season #`
level: AniDB models each season as its own anime (a franchise member), so a
single series name is already one season's folder. Both the series-name and
filename components are sanitized.

Cache-only files (those with a download-cache row, i.e. not yet in a media
root) are flagged in the playlist pane with a dim **`temp`** marker in its
own table column (reserved only while some row is cache-only; the playlist
renders as a table — filename, `temp`, watch state — so an over-long
filename truncates rather than pushing the tag columns off the pane); `A`
only acts on such rows. Archiving moves the file into the library, so the
marker clears — that is the success feedback. Both success and failure
also post a local-only system line to the chat pane ("Archived …" / "Archive
failed (…): …"); these notices are local, not synced.

**A local copy trumps the download.** A file being fetched from peers can
land locally through another channel mid-transfer (e.g. a bittorrent
download racing the prefetch). The library walk (stat-only, so it runs
even while transfers defer scan *hashing*) spots a new file bearing
the name of an unmet playlist entry and resolves it immediately — resolve
hashes outside the scan deferral, and a copy still being written lands in
the mismatch re-check, verifying seconds after the write finishes.
A verified copy cancels the peer download (sources are told to drop our
in-flight chunk requests; the partial cache file is deleted) and the entry
resolves Ready at the local path. The scan also adopts by **hash**: a
matching file under a *different* filename, invisible to the name-based
walk trigger, is adopted when its scan hash comes in. Every "a local
copy turned up" channel — resolve, scan adoption, and a completed
[browse import](#bittorrent-downloads) — funnels through one adoption
seam, so none can skip the cancel. A browse import cancels the peer
download *before* placing its payload in the cache (both share the
hash-addressed cache path), and an import of a file **already held
under a media root** finishes against the library copy instead of
demoting it to a retention-evictable cache row.

**Pre-fetching**: a client with downloading enabled wants **every unwatched
playlist entry** local, plus now-playing itself regardless of the watched
flag (a rewatch is a watch), so next week's episode -- and the whole queue
behind it -- is usually local before the session starts. Fetch order is
**anchored at now-playing**: entries after it in playlist order, nearest
first, then entries behind it (nearest-first), lowest priority. The
ordering acts at the **chunk level** (network-design.md, Scheduling): the
per-source request window is one shared budget filled in this order, so a
now-playing advance or a playlist edit re-targets the running transfers
within a tick -- no cancels, no restarts. A **watched** entry queued ahead
of now-playing does not prefetch; its rewatch fetch starts when it becomes
now-playing. An interactive client **skips auto-download** for entries
whose series it has marked **NotWatching** -- no point fetching a show
you've opted out of. **Maybe** (the default) and **Watching** entries are
fetched normally; a NotWatching file that is already local still loads
(you can mute), it is just never fetched. Seeders fetch everything, in the
same anchored order with watched back-catalog last (see
[Client Roles](#client-roles)).

### BitTorrent Downloads

BitTorrent exists in DessPlay for exactly one thing: the Playlist
pane's explicit **browse search** (`n`) — a user types a query, picks a
release by hand, and it downloads in the background. Missing playlist
files are **never** fetched via torrent: the peer relay is the only
automatic fetch path. (An earlier torrent-first automatic path was
removed in 2026-08 once the peer transfer matured — for rare files a
manual search plus a full BT client covers the gap.) DessPlay is not
primarily a torrent client, and the design keeps its torrent footprint
deliberately session-scoped: nothing torrent-related survives a
restart.

The feature is gated behind the **BitTorrent downloads** setting
(`torrent_enabled`, default **off**) — the engine opens ports and joins
the DHT, so it never starts unless the user opted in. The setting's
lifecycle is **asymmetric**: *enabling* applies at startup (the engine
is constructed then or never), but *disabling* applies **immediately** —
saving the setting removes every seeding torrent (payload files
deleted) and cancels pending imports. This is the mid-session escape
hatch for a saturated uplink (torrent traffic can drown CRDT sync; the
health line's suggestion points here). Cached copies of *completed*
imports are untouched — they were hardlinked into the hash-addressed
cache at verification. Known limitation: the librqbit session and its
DHT socket stay alive until restart — bounded chatter, no payload
traffic. **Seeders run no torrent path at all** — the browse import is
an interactive feature, and a file nyaa can supply makes the seeder
redundant; its job is the rare, peer-only files.

**Search.** The browse search queries nyaa.si's Anime category,
inspects at most the first 20 RSS entries in feed order, and fetches
their torrent metainfo before display so multi-file batches are
excluded rather than failing after selection. Only torrents with at
least one seeder and exactly one safe payload file are listed, with
filename, exact size, release title, and seeder count.

**Import lifecycle.** The selected torrent has no ed2k identity yet, so
it remains a **local pending import** while it downloads into its own
`<cache>/torrents/import-N/` directory — it is not a playlist entry,
does not advertise availability, and never gates anyone. After
completion its single payload is ed2k-hashed, hardlinked into the
ordinary hash-addressed cache (`<cache>/<ed2k>`, block hashes cached
for serving), and only then added to the shared playlist — from which
point it behaves exactly like a completed peer download. A failed or
cancelled import stays a local notice and never creates a provisional
shared entry.

**Session-only seeding.** A completed import keeps seeding from its
import directory — a session typically lasts long enough to clear 1:1
on a release worth importing — until the app closes, the cached file is
evicted (retention or a lost local copy), or the setting is disabled;
upload is capped by the existing `upload_limit` setting. Seeding
deliberately does **not** resume on the next launch: "the video player
is seeding last week's torrents" is unexpected behavior for something
that isn't primarily a torrent client. Accordingly the engine runs
with no persistence, nothing about a torrent is recorded in SQLite,
and at startup the file actor sweeps everything under
`<cache>/torrents/` — abandoned import payloads and any prior
version's leftovers — sparing only a directory that still hosts a
registered cache file (the rare failed-hardlink fallback, where the
cached copy lives in the import dir itself).

The engine is librqbit, embedded (one session per process, rooted at
`<cache>/torrents/`, DHT enabled). A session that fails to start
disables the torrent path with a warning; the relay path still works.

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
and because the user deliberately loaded that exact file. (Either way a
mismatched mapping is never *served*: the holder answers peer
solicitations with a definitive `CannotServe`, so downloaders drop it as
a source rather than re-asking forever -- see network-design.md, Peer
Messages.) dessplay learns
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

**Mismatch re-check.** A name-matched file that fails the hash is
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
- **VLC**: Via embedded Lua TCP script. Whether VLC support lands is an
  open scope decision; mpv is the primary target throughout.

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
  other

### Events from Player

- Position updates (polled or subscribed)
- Pause/unpause events (distinguished: user-initiated vs programmatic).
  An observed pause is followed by a `get_property time-pos` query, and
  the reply re-anchors the client's position estimate on the frame mpv
  actually stopped at — otherwise the wall-clock-extrapolated estimate
  counts the observation's in-flight window as phantom playback, and a
  paused mpv emits no further `time-pos` changes to correct the
  overshoot.
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
  held may be stale (the file moved between media roots).
- Exit (clean or crash)

The user/programmatic distinction is made on **our** side — mpv does not
flag event origins. The player actor tracks what it commanded and
swallows matching observations as echoes (architecture.md, PlayerActor);
because correction is observe-and-correct rather than locally enforced,
a misattributed echo self-heals on the next derived-state round trip. The
`path` observation uses the same model: an observed path equal to the one
we last commanded (including the placeholder PNG) is our echo and is
swallowed; any other is the user's.

**File attribution is evidence-based.** `loadfile` is asynchronous:
after a load is commanded, mpv stays on — and keeps reporting positions,
seeks, even the EOF of — the *previous* file until the new one actually
opens, and on a slow machine (cold NAS, heavy mpv scripts) that window
is long. mpv's events carry no file identity, so the actor pairs each
file-attributed observation (position, seek, EOF, duration) with the
file it *commanded* — which already names the new file during that
window. The observed `path` property closes it: mpv's IPC event stream
is ordered, so the actor accepts file-attributed observations only while
the last observed path equals the commanded one; observations in the gap
belong to the old file and are dropped. The same gate covers outgoing
**drift correction**: authority samples are ignored (and the drift
controller's state reset) while the player is off the commanded file, so
a mid-load window — or a file the user dragged in themselves — is never
slewed or hard-seeked. This is what makes the
`PlaybackPosition` file tag (Playback Rules, drift correction)
trustworthy at its source. Two carve-outs: the load-*failure* report —
a file that fails to open may never produce a path observation at all,
so gating it would suppress the report entirely; a stale one merely
re-resolves the file (wrong but self-healing, the safe direction) — and
the *echo accounting* of a programmatic seek, which is consumed even
when the echo arrives gated-out (it is our own seek; leaving it
outstanding would swallow the user's next genuine seek as a stale echo).
Only the user-seek/debounce half of seek handling sits behind the gate.

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
  (No speaker coloring here -- the lines stay uniformly dim.) When the
  persisted `subtitle_speaker_names` setting is on and the cue carries an ASS
  Name, the body is prefixed `Name: `.
- **Separate pane**: the chat area is split horizontally and the lower
  portion shows recent subtitle lines, **newest first (on top)** so the
  freshest line sits next to the chat input box just below it. The local UI
  tracks each named speaker within the **inclusive rolling five-minute
  wall-clock window** and assigns a stable slot while active. Slots do not
  change while speakers remain active; after more than five minutes without a
  cue, a speaker expires and its slot can be reused. A one-second UI clock
  tick advances this window even during a quiet scene. Speaker names remain
  hidden by default because character names can be spoilers; the persisted
  `subtitle_speaker_names` toggle can opt into a `Name: dialogue` prefix. The
  prefix and dialogue use the same line style. The `MM:SS` timestamp prefix
  stays dim.

  On a true-color terminal there is no application-level speaker cap. Each
  slot extends a deterministic HSLuv palette: from a candidate batch it keeps
  the RGB color whose nearest earlier color is farthest away in CIEDE2000.
  Quantizing before comparison and caching the progressive prefix keeps old
  slots stable while continuing to improve separation as the set grows. The
  colors are built against the explicit dark background and regression-tested
  for at least 4.5:1 text contrast; practical prefixes through 256 speakers
  also have explicit perceptual-distance bounds. On a limited-color terminal,
  speaker names retain the existing
  deterministic hash into the same finite ten-color application palette used
  for usernames; colors can therefore collide or repeat. If the active set
  exceeds ten, the persisted `subtitle_speaker_overflow` policy chooses either
  **Reuse colors** (default/backward-compatible; continue that hashing) or
  **Disable colors** (remove all speaker identity and render every line
  uniformly dim until the active set falls back within capacity).
  The `subtitle_speaker_colors` master setting (default on) remains above this
  policy: when off, all lines are uniformly dim on every terminal. Intermixed
  mode is always uniformly dim.

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
shrink-back is dropped as a redundant re-show (without this, a brief
interjection overlapping a stable line duplicates when it clears). An
exact repeat is the degenerate case and collapses too. A multi-line cue arrives newline-separated; since the log
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
retention, archive subdirectory policy, upload limit, subtitle mode, subtitle speaker names, subtitle
speaker colors, the limited-terminal speaker-color overflow policy, the
commentary-marquee display mode, auto-download, BitTorrent
downloads, IRC bridge settings -- enabled, server, TLS, channel -- and the
AI commentary settings -- Anthropic token, comment interval) live in
the same SQLite database
and are edited through the settings screen. The password is stored in plaintext
— consistent with the threat model below. Command-line flags and environment
variables override stored settings at runtime but are never persisted.
Seeders and the rendezvous server store no settings at all; they are
configured purely by flags/environment (systemd services on NixOS).

### Schema

Versioned via `PRAGMA user_version`; migrations are append-only. All
tables are `STRICT`. Timestamps are unix milliseconds, caller-supplied
(storage never reads the clock — keeps tests deterministic).

The stored `crdt_state` blob carries a **tagged envelope**: a 4-byte
magic (first byte 0xFF, which no untagged postcard state can start with)
plus the protocol version, ahead of the postcard body — so a blob names
its own layout instead of being identified by trial decode. Exactly one
**untagged** legacy layout (protocol v6, pre-envelope) is still decoded
and migrated forward, and tagged versions whose state layout is
byte-identical to the current one (a wire-only protocol bump; today v7
and v8) decode via an explicit compatible-list arm; a tagged blob with
any *other* version is refused outright rather than guessed at (a
deliberate migration adds an explicit decode arm instead). This matters most for the *server*, which is
authoritative and cannot re-sync its lost state from anyone (it backs up
the whole database before first persisting a migrated blob); an
interactive client can fall back to dropping an unreadable blob and
re-syncing from the server. See docs/sync-state.md, Snapshot Storage.

**Client** (`$XDG_DATA_HOME/dessplay/dessplay.db`):

| Table | Contents |
|-------|----------|
| `settings` | Key-value settings (username, server, password, player, ready_on_startup, cache_retention, upload_limit, subtitle_mode, subtitle_speaker_names, subtitle_speaker_colors, subtitle_speaker_overflow, marquee_mode, auto_download, archive_subdirectory, torrent_enabled, irc_enabled, irc_server, irc_tls, irc_channel, anthropic_token, commentary_interval) |
| `media_roots` | Ordered media roots; position 0 is the download target |
| `crdt_state` | Latest snapshot per room (epoch + version-tagged postcard blob); single `'default'` room in v1 |
| `watch_history` | Personal watched files: hash → series id/name, filename, watched_at |
| `cache_entries` | Download-cache bookkeeping: hash → path, size, last_access; an index, reconciled against disk at startup (stale rows pruned; row-less hash-named files >1 week old swept) |
| `hash_cache` | Path → ed2k root + per-block hashes, keyed by (mtime, size), plus nullable owning media root; skips re-hashing unchanged files and doubles as the library index |
| `library_roots` | Durable root lifecycle (`vanished_at`, `removed_at`); hides disconnected roots indefinitely and expires removed-root index rows after seven days |
| `manual_mappings` | Playlist hash → user-picked local path; also holds out-of-root hash-adds (paste/browse), registered in place so they stay servable across restarts |
| `series_map_dirs` | Per-series last-used mapping directory (`anidb:<id>` / `name:<parsed>`) |
| `tofu_fingerprints` | Pinned server cert fingerprints; write-once (replacing requires explicit forget) |

**Server** (`$XDG_DATA_HOME/dessplay-rendezvous/rendezvous.db`,
`--db-path` to override):

| Table | Contents |
|-------|----------|
| `crdt_state` | The authoritative snapshot (epoch + version-tagged postcard blob) |
| `chat_archive` | Full chat history, archived before compaction trims the replicated GList; unique on (timestamp, sender, text), mirroring GList dedup |
| `anidb_queue` | FILE validation queue: hash, size, filename, mtime (anchors the re-validation ladder on the file's real age), `series_hint` (title-like containing-directory name; the AniDB-miss fallback series name, so episodes group by folder), attempt bookkeeping, `next_attempt` scheduling (`i64::MAX` = settled tombstone) |
| `anime_queue` | ANIME (relations-walk) queue: aid, attempt bookkeeping; the graph fills in over hours and must survive restarts |
| `anidb_titles` | The anime-titles dump (aid, kind, lang, title); backs local name search |
| `kv` | Bookkeeping (e.g. the titles dump's last fetch time) |
| `known_users` | Every username ever seen, with a last-seen timestamp; updated on connect/disconnect, survives restarts unlike the in-memory peer registry |

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

- Automating The List's "this week's episode is out" flag (possibly via AniDB
  episode air dates).
- Direct client-to-client connections (with or without hole punching) as a
  transfer optimization, slotted in beneath the `send(peer, message)`
  interface. Cut from v2: the relay-through-NAS path makes them unnecessary.
- [AI Commentary marquee](#ai-commentary-the-marquee) refinements: keying
  the commentator per-user, and marquee sources beyond commentary (the
  register is deliberately generic).
- A GUI, deferred until everything else is done. The web-renderer approach
  (ui-architecture.md) was dropped for lack of interest, so the mechanism
  is open.

Dropped in the 2026-08-17 usage triage (see plan.md for reasoning):
interleaving Intermixed subtitles by in-video timestamp, the
bitrate-aware unpause rule, the web frontend, disk-aware prefetch depth,
and many-peer choking.
