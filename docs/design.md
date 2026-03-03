# DessPlay Design Document

A synchronized video player for watch parties. Terminal-first, peer-to-peer, built for reliability over flaky connections.

## Table of Contents

1. [User Experience](#user-experience)
2. [TUI Layout](#tui-layout)
3. [Network Protocol](#network-protocol)
4. [File Management](#file-management)
5. [Player Integration](#player-integration)
6. [Data Storage](#data-storage)
7. [Architecture](#architecture)

---

## User Experience

This section describes the full workflow from a user's perspective.

### First Launch

1. **Launch DessPlay**
   - From the terminal: `dessplay`
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

### Connecting to Friends

1. DessPlay connects to the rendezvous server (QUIC, TOFU certificate trust)
2. The rendezvous server establishes a shared clock, via an NTP-style protocol
   using the rendezvous server as the authoritative clock.
2. The rendezvous server provides peer addresses and the client's observed address
3. Direct peer-to-peer QUIC connections are established to discovered peers
   We assume peers are accessible by public IPv6 address, but firewalled. The
   rendezvous server assists in hole punching.
4. If direct connection fails, traffic relays through the rendezvous server
5. Connected users appear in the **Users pane** (top-right area)
6. Connection happens automatically on launch; no manual action needed

### Adding Files to the Playlist

**From the Series pane:**
1. Press `Tab` to focus the **Series** pane (top-right)
2. The pane has two modes, toggled with `m`:
   - **Recent Series** (default): franchises sorted by unwatched → recency → alphabetical
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
   - **Known series** (you've previously watched a file from the same source
     directory): the file is marked **Missing** — this blocks playback,
     because you probably should have this file
   - **Unknown series** (no watch history from that directory): you are set to
     **Not Watching** — a generated placeholder PNG is loaded into your player
     showing the current state.
4a. You can manually map to a different file:
   - Select the red entry, press a key (ctrl-m) to open browser
   - Browser opens to the directory most recently used for files from that series
   - Files are sorted by edit distance to the target filename
4b. You can manually set yourself to "not watching" on a file that's Missing
   (e.g. a known series but you don't have this episode yet). This clears the
   "missing from known series" block
4c. By default: The file will is retrieved from
    peers using a bittorrent-like protocol. It is kept in a temporary directory
    until at least 50% of the file has been watched, at which point it is
    moved into [Series name]/[Season #]/[Original filename] in the download root,
    aka. the topmost media root.

### User States

Each user has a flag describing their personal state of readyness.
The default value for this can be set on the settings screen.

This can take three values:
- Ready: The user is ready and waiting.
- Paused: The user is *not* ready — either stepped away or manually paused.
- Not watching: The user is intentionally skipping the current file.


### File State

Each user has a flag describing their *ability* to play the current file.
It can have one of three values:

- Ready: The hash matches, the file is loaded, and it can be unpaused as desired.
- Missing: The file doesn't exist, or the hash is mismatched, and none of the step
  4 options from file matching have been performed.
- Downloading: The user's client is actively retrieving the file from the other
  clients. Unpausing is conditional: To unpause, their download speed must be higher
  than the file's computed bitrate, *and* at least 20% of the file must be downloaded.


### Ready States

Each user has a ready state, shown by state & color in the Users pane.
Their ready state is decided by a combination of the above; this only exists in the UI.

| State | Color | Meaning |
|-------|-------|---------|
| Ready | Green | Ready & Ready |
| Paused | Red | Paused & Any |
| Not watching | Gray | Not watching & Any |
| Downloading | Green | Ready & Downloading[complete enough to play] |
| Downloading | Blue | Any & Downloading |

The OSD on the video player shows a summary: Which users are unready (in any form), how many users are connected.

**How states change:**

- **On join**: User State starts as Ready or Paused (depending on "Ready on startup" setting); File State depends on whether the file was found locally
- **Missing file (unknown series)**: User State → Not Watching; File State → Missing; placeholder PNG loaded into player
- **Missing file (known series)**: File State → Missing (blocks playback — you probably should have this file)
- **Missing file (downloading enabled)**: File State → Downloading; placeholder is updated with download progress
- **Manual pause** (in player): User State → Paused
- **Attempt unpause** (in player): User State → Ready; unpauses if all users permit it
- **Mark "not watching"**: Toggle on playlist item, sets you to Not Watching for that file (clears "missing from known series" block)

### Playback Rules

1. **Play** only proceeds when every user is Ready or Not Watching, and their File State permits playback
2. If you press play in your player but someone is Paused or has a Missing file:
   - Your player is immediately re-paused
   - You are marked Ready (you tried!)
3. When someone pauses, everyone pauses
4. When someone seeks, everyone seeks
5. Sync tolerance is 3 seconds; no seek triggered for smaller drift
6. Seeks are debounced (1500ms) — only broadcast after the user stops scrubbing
7. **EOF** advances the synced now-playing pointer to the next playlist entry.
   Files are **not** removed from the playlist on EOF — they remain visible
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
  - `/exit`, `/quit`, `/q`, ctrl-c — quit DessPlay

### Watching a Series

Typical evening flow:

1. Launch DessPlay, it connects automatically
2. Check Recent Series pane - your ongoing shows are listed
3. Select series, add next episode to playlist
4. Wait for friends' names to turn green
5. Anyone presses play, episode starts
6. Chat during the episode (appears on video)
7. Episode ends, add next one (or it's already queued)
8. Repeat until bedtime

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
- Right 50%, top: Series (dual-mode: Recent Series / All Series)
- Right 50%, middle: Users
- Right 50%, bottom: Playlist

**Keybinding bar:** 1-line context-sensitive bar at the very bottom. Shows
available actions for the currently focused pane (e.g. Chat shows
`Tab | Enter | Esc | Ctrl-C`, other panes show `Tab | Ctrl-C`).

**Focus cycling:** `Tab` cycles through Chat, Recent Series, Playlist

**Mouse support:** Click to focus panes, scroll, select items (if convenient to implement)

### Keyboard Shortcuts

| Key | Context | Action |
|-----|---------|--------|
| `Ctrl-C` | Any | Quit |
| `Tab` | Any | Cycle focus: Chat → Series → Playlist → Chat |
| `Enter` | Chat | Send message (or execute `/command`) |
| `Esc` | Chat | Clear input |
| `Backspace` | Chat | Delete character before cursor |
| `Delete` | Chat | Delete character after cursor |
| `Left` / `Right` | Chat | Move cursor |
| `Ctrl-Left` / `Ctrl-Right` | Chat | Move cursor by word |
| `Home` / `End` | Chat | Move cursor to start/end |
| `m` | Series | Toggle mode: Recent Series ↔ All Series |
| `s` | Series (All mode) | Toggle sort: by title ↔ by year |
| `Enter` | Series | Browse franchise (episode browser or file browser) |
| `Enter` | Episode Browser | Select season / add episode to playlist |
| `Esc` / `Backspace` | Episode Browser | Go back (episodes → seasons → close) |
| `Enter` | Playlist | Play selected entry (or open file browser on [Add New]) |
| `a` | Playlist | Add file (insert after selected entry) |

Note: there is no `q` to quit — too easy to hit while typing in chat.

---

## Network Protocol

### Overview

- **Transport**: QUIC for all peer-to-peer communication
- **Topology**: Full mesh (everyone connects to everyone)
- **Sync model**: Operation-based CRDTs (CmRDTs) — see [sync-state.md](sync-state.md)
- **Wire protocol**: QUIC streams + datagrams, postcard serialization — see [network-design.md](network-design.md)
- **Conflict resolution**: Per-datatype — LWW registers for most state, op log CRDT for playlist, append-only logs for chat

### Rendezvous Server

Located on VPS. Separate binary (`dessplay-rendezvous`). Responsibilities:

1. **Peer registration**: Clients report their presence via QUIC control stream
2. **Peer list distribution**: Clients receive list of other peers
3. **STUN**: Tell clients their observed IP:port in Register response
4. **TURN relay**: Forward datagrams and streams between clients who cannot connect directly

**Authentication**: Password entered on first client launch, sent in plaintext
over TLS-encrypted QUIC. Server configured via `--password-file` or env var.

**TLS**: TOFU (Trust On First Use) — server generates a persistent self-signed
cert; clients store and verify the fingerprint on subsequent connections.

The rendezvous server DOES participate in state sync. Once peers are connected, they communicate directly;
however, the rendezvous server has two crucial jobs:
- **Compaction**: Replaying the CRDT op logs into snapshots and incrementing the epoch counter
  (triggered after 5 minutes with no connected clients).
- **AniDB lookups**: Filling in and updating series/season/episode numbers for files added by clients.

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

| Data | CRDT Type | Transport |
|------|-----------|-----------|
| User State (Ready/Paused/Not Watching) | LWW Register | Datagrams + reliable gap fill |
| File State (Ready/Missing/Downloading) | LWW Register | Datagrams + reliable gap fill |
| Playlist | Op log CRDT (Add/Remove/Move by file ID) | Datagrams + reliable gap fill |
| Chat | Per-user append-only log | Datagrams + reliable gap fill |
| Playback position | Ephemeral LWW (fire-and-forget) | Datagrams only |
| Seek events | Imperative command (debounced) | Datagrams only |
| AniDB metadata | LWW Register (server-authoritative) | Reliable stream |
| Now Playing | LWW Register (singleton) | Datagrams + reliable gap fill |

Playback state (playing vs paused) is **derived**, not synced directly:
the video plays iff every user's User State is Ready or Not Watching, and
their File State permits playback (Ready, or Downloading with sufficient
progress). This keeps position and readiness as independent concerns —
a seek cannot silently overwrite a pause, since they are in different fields.

Playback position is broadcast every 100ms during playback, 1s when paused.
Periodic state summaries (version vectors) are exchanged every 1s to detect
missed operations.

### Chat Protocol

Chat messages include:
- Sender username
- Message text
- Timestamp
- Sequence number (per-user)

Reliability: Chat is a per-user append-only log CRDT (see [sync-state.md](sync-state.md)).
New messages are eager-pushed via datagrams; state vectors in periodic summaries
detect gaps; missing entries are recovered via reliable-stream gap fill.

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

### Parsing files to series/season/episode

We use the AniDB UDP API, with the understanding that the information may be incomplete and/or require later updates. See https://wiki.anidb.net/UDP_API_Definition

Crucially:
- The API is rate-limited. Clients MUST NOT send more than 1 packet every 2 seconds, and also MUST NOT send more than 1 packet every 4 seconds with a burst of 60.
- Server-throttled packets are counted against this rate limit. Throttling is unpredictable; on a missing response, the client MUST wait 5 seconds before retrying.
- Files SHOULD be re-validated on a reasonable schedule: Every 30 minutes if it is less than a day old, every 2 hours if it's less than a week, and so on. Files older
  than 3 months do not need to be re-validated. This is only true when AniDB fails to return data for a file.
- Files which *do* have data should still be re-validated, but MUST NOT be re-validated more than once per week.
- The code needs to account for the client being turned off most of the time; the validation queue needs to be in SQLite, not done by way of sleeps.
- The client id is "dessplay".
- All commands besides LOGIN require first logging in.

All interaction with AniDB is done by the rendezvous server, not the clients.

CRDT type: LWW Register keyed by `ed2k_hash`, value is `None | JSON`.

State flow:
1. One or more clients create a register for a given file hash (initial value `None`).
2. The rendezvous server queries AniDB and overwrites the register with the result.

Conflict resolution: Server-authoritative (server writes always win).


### Manual File Mapping

When explicitly invoked:

1. User selects the missing item in playlist
2. Opens file browser
3a. If series & episode number is known, and there is a local match (different filename, same series, season & episode): Cursor is placed on this file.
3b. If series is known, and the user has previously used the map function for this series: Browser opens to the most recently used directory.
3c. Otherwise, the browser opens to the list of media roots.
4. User selects correct file
5. Mapping stored locally

### Content Hash

Before playback can unpause:

1. Compute ed2k hash
2. Compare with other Ready users

If hash mismatch: File State is set to Missing, cannot participate until resolved.
File mtime is stored in memory. Hash is recomputed whenever mtime changes, until there
is a match.

This is skipped for manually-mapped files (user explicitly chose a different file).

### Watch Tracking

- A file is "watched" when 85% of its duration has been played
- Tracked per-file in local database
- Used for:
  - Sorting "Recent Series" (most recently watched on top)
  - Filtering "unwatched files" in series browser
  - **Known series detection**: a series is "known" if you
    have previously watched any file from it. This affects missing file
    behavior — see [File Matching](#file-matching)

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
- **VLC**: Via embedded Lua TCP script; v2 only

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
- `get_property time-pos`: Query current position
- `show-text <message>`: Display OSD message

### Events from Player

- Position updates (polled or subscribed)
- Pause/unpause events
- EOF (file ended)
- Exit (clean or crash)

---

## Data Storage

### SQLite Database

Location: `$XDG_DATA_HOME/dessplay/dessplay.db` (typically `~/.local/share/dessplay/`)

Uses `rusqlite` with `bundled` feature. All CRDT state (snapshots and op logs) is
persisted per-room so it survives full disconnects. On startup, the stored state
is loaded and passed to the sync engine as initial state. The current epoch is
also stored so the client can detect stale state on reconnection.

### Schema

TBD

---

## Architecture

### Component Overview

```
+------------------+     +------------------+
|    TUI Layer     |     |  Player Bridge   |
|                  |<--->|  (mpv/vlc IPC)   |
+--------+---------+     +--------+---------+
         |                        |
         v                        v
+------------------------------------------+
|              Core State                   |
|  (playlist, positions, ready states)     |
+--------+---------------------------------+
         |
         v
+------------------+     +------------------+
|   Network Layer  |     |   Storage Layer  |
|  (QUIC, STUN/TURN)|    |   (SQLite)       |
+------------------+     +------------------+
```

### File structure

TBD

### Key Dependencies (tentative)

TBD

### GUI Future-Proofing

The architecture separates concerns to allow a GUI frontend later:

- **Core** is independent of TUI
- **Player Bridge** is independent of UI
- **Network Layer** is independent of UI

A GUI could replace only the TUI layer, reusing everything else.

---

## Security / Threat Model

Authentication uses a password entered on first launch and stored in the local
database. The password is used to derive HMAC keys for the rendezvous protocol.
Anyone with the password can connect. This is acceptable for the intended use
case (small friend groups) but should be documented clearly.

- **Identity**: Users are identified by self-chosen nicknames. There is no
  cryptographic identity — users trust each other not to impersonate.
- **Confidentiality**: Peer-to-peer traffic is encrypted via QUIC's built-in
  TLS 1.3. Rendezvous protocol messages are authenticated with HMAC but not
  encrypted (they contain only addresses and room names, no sensitive content).
- **Integrity**: No message authentication beyond HMAC. A peer with the
  password could send forged state updates.
- **Availability**: Any peer can pause playback for everyone. This is by design.

For v1, this is acceptable. Future improvements could include:
- Session invite codes (short-lived tokens instead of shared password)
- Per-user key pairs for identity and message authentication

---

## Key Definitions

- **FileId**: The ed2k hash of a file's contents. All playlist operations,
  file state tracking, and content verification use this as the unique
  identifier for a file. This means a file must be hashed before it can be
  added to the playlist.

- **Rooms**: A rendezvous server can in theory host multiple rooms. For v1,
  there is a single implicit room per server. Multi-room support is future work.

## Open Questions

TBD

---

## Future Plans

Besides those already mentioned:
- A subtitle pane, showing recent subtitles from the video. Optionally fused with the chat pane.
- Automated, sharded download from peers for missing files.
