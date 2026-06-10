# DessPlay Design Document

Last updated: 2026-06-10

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
2. **Settings screen** appears (first run only):
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
  Media roots can be reordered with ctrl-j/ctrl-k.

Optional settings (sensible defaults, editable later):
- Cache retention (duration; `0` = delete watched downloads at end of session,
  `infinite` = keep everything; see [Download Cache](#download-cache-and-retention))
- Upload limit (bytes/sec cap for serving files to peers; default unlimited)
- Subtitle pane (on/off; default off)

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
2. The pane has two modes, toggled with `m`:
   - **Recent Series** (default): franchises sorted by unwatched > recency > alphabetical
   - **All Series**: same data, sorted by title or year (toggle with `s`)
3. Related anime are grouped into **franchises** using AniDB's relations graph
   (sequel, prequel, side story, etc.). Each franchise shows as one entry.
4. Press `Enter` on a franchise:
   - **Single-season franchise**: opens the file browser in the series directory,
     cursor on the next unwatched episode
   - **Multi-season franchise**: opens the **Episode Browser** modal showing
     seasons (franchise members). Select a season to see its episodes.
5. In the Episode Browser, press `Enter` on an episode with a local file to add
   it to the playlist. Press `Esc`/`Backspace` to go back.
6. Sort mode for All Series is persisted across sessions.

**From scratch:**
1. Press `Tab` to focus the **Playlist** pane (bottom-right)
2. Press `a` to add a file
3. Navigate your media root directories
4. Select file to add. (Enter)

**Reordering:**
1. Focus the **Playlist** pane
2. Use `Ctrl-j` / `Ctrl-k` to move the selected item down/up

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
     showing the current state.
4a. You can manually map to a different file:
   - Select the red entry, press a key (ctrl-m) to open browser
   - Browser opens to the directory most recently used for files from that series
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

1. **Per-series watch preference** (`Map<AniDbSeriesId, LWWReg<SeriesWatchState>>`):
   Each user can mark themselves as NotWatching for a specific AniDB series.
   When the currently playing file belongs to a series the user has marked as
   NotWatching, their state is derived as NotWatching.

2. **Manual override** (`LWWReg<Option<ManualState>>`): The user can manually
   pause (stepping away), which overrides the series-based state. The override
   is cleared when the user explicitly resumes. `ManualState` is
   `Paused | Away { set_by: UserId }`.

**Away**: any user can mark *another* user as Away (`/afk <name>` in chat, or
`a` on a user in the Users pane) -- for when someone walks off without quitting
and would otherwise block playback forever. Away behaves like Not Watching for
playback gating, and is displayed with attribution ("away, set by Baughn"). Any
input from the marked user's client (keypress, unpause) clears it back to
normal. With five trusted friends, no permission system is needed.

Derived states:
- **Ready**: No manual override, and the current series is not marked NotWatching
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

### Ready States (UI Display)

Each user has a ready state shown by state & color in the Users pane.
Their ready state is decided by a combination of the above; this only exists in the UI.

| State | Color | Meaning |
|-------|-------|---------|
| Ready | Green | Ready & Ready |
| Paused | Red | Paused & Any |
| Away | Gray | Away & Any (shows who set it) |
| Not watching | Gray | Not watching & Any |
| Downloading | Green | Ready & Downloading [complete enough to play] |
| Downloading | Blue | Any & Downloading |

Departed users (see [Presence](#presence)) are shown on a dim "departed" line.
Seeders are not listed as users; they appear on a separate dim "seeders:" line.

The OSD on the video player shows a summary: Which users are unready (in any form),
how many users are connected.

**How states change:**

- **On join**: User State starts as Ready or Paused (depending on "Ready on startup"
  setting); File State depends on whether the file was found locally
- **Missing file (unknown series)**: User State -> Not Watching; File State -> Missing;
  placeholder text loaded into player
- **Missing file (known series)**: File State -> Missing (blocks playback)
- **Missing file (downloading enabled)**: File State -> Downloading; placeholder is
  updated with download progress
- **Manual pause** (in player): Manual override -> Paused
- **Attempt unpause** (in player): Manual override -> None; unpauses if all users permit
- **Marked Away** (by another user): Manual override -> Away; cleared by any
  input from the marked user's client
- **Mark "not watching"** on series: Series preference updated; clears "missing from
  known series" block when applicable

### Playback Rules

1. **Play** only proceeds when every **present** user is Ready, Away, or Not
   Watching, and their File State permits playback. Presence is defined in
   [Presence](#presence); departed users and seeders never gate playback.
2. If you press play in your player but someone is Paused or has a Missing file:
   - Your player is immediately re-paused
   - You are marked Ready (you tried!)
3. When someone pauses, everyone pauses
4. When someone seeks, everyone seeks (via seek authority; see [sync-state.md](sync-state.md))
5. **Drift correction** relative to the seek authority's position uses three bands
   (thresholds configurable in one place; defaults below):
   - **< 100ms**: ignore
   - **100ms - 3s**: slew -- adjust playback speed by up to ±2% (mpv `speed`
     property, pitch-corrected, invisible to the viewer) until converged
   - **> 3s**: hard seek
6. Seeks are debounced (1500ms) -- only broadcast after the user stops scrubbing
7. **EOF** advances the synced now-playing pointer to the next playlist entry.
   The server initiates this (it is the authoritative entity for "file ended"):
   clients whose player reaches end-of-file send an `EofReached { file }` report
   to the server; when the server receives the first report matching the current
   now-playing file from a present, watching user, it marks the file watched,
   advances now-playing, and takes seek authority. Later duplicate reports no
   longer match now-playing and are ignored, making the transition idempotent.
   Files are **not** removed from the playlist on EOF -- they remain visible
   in muted colors as play history. Users can select any entry with Enter to
   set it as now-playing.

### Before Playback Starts

Before unpausing is allowed, DessPlay verifies file contents match:

1. Compute ed2k hash of the local file
2. Compare hashes across all Ready users
3. If mismatch: unpause is blocked, File State is set to Missing

This prevents sync issues from different encodes/versions.

### Chat

- Type in the chat input (always visible at bottom of chat pane)
- Press Enter to send
- Messages appear in the chat pane AND as OSD in the video player
- System messages (joins, disconnects, state changes) appear in chat
- Text commands start with `/`:
  - `/exit`, `/quit`, `/q`, ctrl-c -- quit DessPlay
  - `/afk <name>` -- mark another user as Away (see User States)

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
- **Storage** follows the same cache-retention setting as interactive clients
  (see [Download Cache](#download-cache-and-retention)). A NAS seeder sets
  retention to `infinite`; "should this be archived into the media library?"
  remains a manual, human decision via the archive action on any interactive
  client that shares the filesystem -- or simply by moving the file.

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
| **Lost** | 30s without traffic (QUIC idle timeout; clients keep-alive every 10s, and position updates double as liveness) | Everyone pauses; system message in chat |
| **Departed** | 60s without traffic | Removed from gating and from the active Users list (shown on a dim "departed" line). Playback **stays paused** -- resuming is a human decision; the usual response is to switch shows. No auto-unpause. |

Additional rules:

- **Brief glitches (< 30s) are invisible.** Everyone keeps watching; the
  shared clock keeps players aligned, and slew correction absorbs small drift
  on recovery.
- **Graceful quit** (`/quit`, Ctrl-C): the user is removed immediately
  (no Lost stage). If playback was running, it pauses -- leaving mid-episode
  should not be silent. At session end this is a no-op.
- **Return**: a reconnecting user re-enters as Present, syncs state, and is
  gated normally again. Playback does not auto-resume.
- **Seek authority**: if the current seek authority becomes Departed, the
  server takes seek authority.
- Departed users' CRDT state (manual override, file availability) persists
  but is ignored by gating until they return.

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
episode (same EOF transition that marks the playlist entry watched).
`available` is maintained by hand for now; automating it (e.g. via AniDB
episode air dates) is future work.

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
in a small edit modal; linking an unlinked entry opens an AniDB search modal
(search by name, confirm manually -- informal names like "GochiUsa" will not
fuzzy-match reliably).

The `watchers` set wires into the existing per-series watch preference: when
an entry is linked, users *not* in the watchers set get
`SeriesWatchState::NotWatching` for that series, so they never gate playback
on shows they skip.

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

**Subtitle pane (optional):** when enabled (settings toggle or `F2`), the
chat area is split horizontally and the lower portion shows recent subtitle
lines from the local player (see [Player Integration](#player-integration)).
A future refinement may fuse subtitles into the chat log, interleaved by time.

**Keybinding bar:** 1-line context-sensitive bar at the very bottom. Shows
available actions for the currently focused pane. Derived automatically from
the active component's keybinding declarations (see [ui-architecture.md](ui-architecture.md)).

**Focus cycling:** `Tab` cycles through Chat, Series, Users, Playlist

**Mouse support:** Click to focus panes, scroll, select items (if convenient to implement)

### Keyboard Shortcuts

| Key | Context | Action |
|-----|---------|--------|
| `Ctrl-C` | Any | Quit |
| `Tab` | Any | Cycle focus: Chat -> Series -> Users -> Playlist -> Chat |
| `F2` | Any | Toggle subtitle pane |
| `Enter` | Chat | Send message (or execute `/command`) |
| `Esc` | Chat | Clear input |
| `Backspace` | Chat | Delete character before cursor |
| `Delete` | Chat | Delete character after cursor |
| `Left` / `Right` | Chat | Move cursor |
| `Ctrl-Left` / `Ctrl-Right` | Chat | Move cursor by word |
| `Home` / `End` | Chat | Move cursor to start/end |
| `m` | Series | Cycle mode: Recent Series -> All Series -> The List |
| `s` | Series (All mode) | Toggle sort: by title <-> by year |
| `Enter` | Series | Browse franchise (episode browser or file browser) |
| `Enter` | Series (List mode) | Jump to next episode / open entry |
| `e` | Series (List mode) | Edit entry (modal) |
| `l` | Series (List mode) | Link entry to AniDB (search modal) |
| `Enter` | Episode Browser | Select season / add episode to playlist |
| `Esc` / `Backspace` | Episode Browser | Go back (episodes -> seasons -> close) |
| `a` | Users | Mark selected user as Away (or clear an Away you set) |
| `Enter` | Playlist | Play selected entry (or open file browser on [Add New]) |
| `a` | Playlist | Add file (insert after selected entry) |
| `d` | Playlist | Remove selected entry |
| `A` | Playlist | Archive selected cached file into the download root |
| `Ctrl-m` | Playlist | Manually map selected entry to a local file |

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
5. **Compaction**: Scheduled daily (default 12:00 server-local time, configurable
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
| Playlist | `Map<Ed2kHash, MVReg<Lww<Option<PlaylistFileState>>>>` | `Identifier`-based ordering; includes size and duration; `None` = removal tombstone (purged at compaction) |
| Watched flags | `Map<Ed2kHash, MVReg<Lww<bool>>>` | Server-only writes (at EOF) |
| Now Playing | `MVReg<Lww<Option<Ed2kHash>>>` | Standalone register; server writes on EOF |
| Seek Authority | `MVReg<Lww<ActorId>>` | Standalone register; last seeker is position authority |
| Series preference | `Map<(UserId, AniDbSeriesId), MVReg<Lww<SeriesWatchState>>>` | Compound key |
| Manual override | `Map<UserId, MVReg<Lww<Option<ManualState>>>>` | Per user; Away writable by anyone |
| File availability | `Map<(UserId, Ed2kHash), MVReg<Lww<FileAvailability>>>` | Compound key |
| AniDB metadata | `Map<Ed2kHash, MVReg<Lww<Option<AniDbMetadata>>>>` | Server-authoritative |
| Series relations | `Map<AniDbSeriesId, MVReg<Lww<SeriesRelations>>>` | Server-authoritative; franchise graph |
| The List | `Map<ListEntryId, MVReg<Lww<SeriesListEntry>>>` | Any peer; never pruned |
| List next-ep | `Map<ListEntryId, MVReg<Lww<NextEpState>>>` | Any peer; server auto-advances |
| Lookup requests | `GSet<FileHashInfo>` | Clients insert; cleared on compaction |
| Chat | `GList<ChatMessage>` | Grow-only ordered list; trimmed on compaction (server archives full history) |
| Playback position | `Map<UserId, MVReg<Lww<PlaybackPosition>>>` | Per user, high frequency, datagram-only transport |

All `MVReg` values wrap `Lww<V>` for last-writer-wins conflict resolution.
`ActorId` type parameters omitted from the table for brevity -- all CRDTs
use `ActorId` as the actor type. See [sync-state.md](sync-state.md) for
the full `Lww<V>` design.

Playback state (playing vs paused) is **derived**, not synced directly:
the video plays iff every present user's state is Ready, Away, or Not
Watching, and their File State permits playback.

### Chat Protocol

Chat messages include:
- Sender username
- Message text
- Timestamp

Reliability: Chat is a `crdts::GList` (grow-only list). New messages are
sent through the server; the CRDT handles ordering and deduplication.

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
(`$XDG_CACHE_HOME/dessplay/files/`). They are never automatically promoted
into a media root.

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
path.

**Pre-fetching**: clients with downloading enabled fetch playlist entries
*ahead* of now-playing (in playlist order) in the background, so next week's
episode is usually local before the session starts. Seeders fetch everything
(see [Client Roles](#client-roles)); interactive clients pre-fetch within
their retention/disk constraints.

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

**Lookup flow:**
1. Clients scan local files, hash them, and insert `FileHashInfo` (hash, size,
   filename) into a `GSet<FileHashInfo>` -- a "please look these up" set.
2. The server drains entries from this set into its AniDB lookup queue.
3. On success: server writes full `AniDbMetadata` (series name, ID, episode).
4. On failure (AniDB doesn't know the file): server writes filename-derived
   metadata (series name parsed from filename, no series ID, no episode number).
5. Either way, the metadata register becomes `Some(AniDbMetadata)` --
   downstream code always has a series name to work with.

CRDT types:
- Lookup requests: `GSet<FileHashInfo>` (cleared on compaction)
- Metadata: `LWWReg<Option<AniDbMetadata>>` keyed by ed2k hash (server-only writes)

See [sync-state.md](sync-state.md) for the full `AniDbMetadata` struct.

**Franchise relations:** grouping series into franchises requires AniDB's
relations graph (sequel, prequel, side story). When a file lookup yields a new
series ID, the server queues ANIME lookups for it and walks its relations
recursively (each hop is another rate-limited request, so the graph fills in
over hours -- fine, it's needed for browsing, not playback). Results are
cached in server SQLite and replicated as the server-authoritative
`SeriesRelations` map. Clients build franchise groupings from this map; files
without a series ID group by parsed series name as a fallback.

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

### Content Hash

Before playback can unpause:

1. Compute ed2k hash
2. Compare with other Ready users

If hash mismatch: File State is set to Missing, cannot participate until resolved.
File mtime is stored in memory. Hash is recomputed whenever mtime changes, until
there is a match.

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

1. **Launch**: When file is selected, spawn player process with file path
2. **Control**: Send play/pause/seek commands via IPC
3. **Monitor**: Read current position, playback state
4. **OSD**: Display chat messages in video window
5. **Crash handling**:
   - First crash: Auto-relaunch, seek to last position
   - Second crash within 30s: Pause globally, notify in chat

### Commands Sent to Player

- `loadfile <path>`: Load video file
- `pause` / `unpause`: Control playback
- `seek <seconds>`: Seek to position
- `set_property speed <factor>`: Slew playback rate for drift correction
  (±2% max; mpv's pitch correction makes this inaudible)
- `get_property time-pos`: Query current position
- `show-text <message>`: Display OSD message

### Events from Player

- Position updates (polled or subscribed)
- Pause/unpause events (distinguished: user-initiated vs programmatic)
- Seek events (distinguished: user-initiated vs programmatic)
- Subtitle text changes (observed `sub-text` property; feeds the subtitle pane)
- EOF (file ended; reported to the server, which owns the transition)
- Exit (clean or crash)

### Subtitle Pane Feed

The subtitle pane is **local-only**: it shows whatever subtitle line the
user's own player is currently displaying, via mpv's `sub-text` property
observation, appended to a rolling log. Nothing is synced -- different
releases or sub tracks per user are fine. Image-based subtitle formats
(PGS/VobSub) expose no text; the pane simply stays empty for those.

---

## Data Storage

### SQLite Database

Location: `$XDG_DATA_HOME/dessplay/dessplay.db` (typically `~/.local/share/dessplay/`)

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
retention, upload limit, subtitle pane) live in the same SQLite database and
are edited through the settings screen. The password is stored in plaintext
— consistent with the threat model below. Command-line flags and environment
variables override stored settings at runtime but are never persisted.
Seeders and the rendezvous server store no settings at all; they are
configured purely by flags/environment (systemd services on NixOS).

### Schema

TBD (Phase 2)

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
  variant — files whose size is an exact multiple of the 9,728,000-byte
  block size include a trailing empty-block hash — for compatibility with
  AniDB FILE lookups. Per-block MD4 hashes are kept alongside the root for
  transfer verification.

- **Rooms**: A rendezvous server can in theory host multiple rooms. For v1,
  there is a single implicit room per server. Multi-room support is future work.

- **ActorId**: Unique identifier for a participant in the CRDT system. Each
  client has one, and the server has a well-known server ActorId used for
  authoritative actions (EOF transitions, seek authority on file change).

---

## Future Plans

- Subtitle pane fused with the chat pane (interleaved by timestamp). The
  standalone subtitle pane is in scope for v2.
- Automating The List's "this week's episode is out" flag (possibly via AniDB
  episode air dates).
- Direct client-to-client connections (with or without hole punching) as a
  transfer optimization, slotted in beneath the `send(peer, message)`
  interface. Cut from v2: the relay-through-NAS path makes them unnecessary.
- Web UI using the same `ViewSpec` approach (see [ui-architecture.md](ui-architecture.md)).
