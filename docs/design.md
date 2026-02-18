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
   - From the web: `dessplay.brage.info`
2. **Settings screen** appears (first run only):
   - Enter your username
   - Choose your player (mpv or vlc; terminal version only)
   - Add media root directories (where your anime/shows live; terminal version only)
3. **Main screen** appears with chat pane, users list, playlist and video library

### Connecting to Friends

1. DessPlay connects to the rendezvous server (QUIC, TOFU certificate trust)
2. The rendezvous server provides peer addresses and the client's observed address
3. Direct peer-to-peer QUIC connections are established to discovered peers
4. If direct connection fails, traffic relays through the rendezvous server
5. Connected users appear in the **Users pane** (top-right area)
6. Connection happens automatically on launch; no manual action needed

### Adding Files to the Playlist

**From the Recent Series pane:**
1. Press `Tab` to focus the **Recent Series** pane (top-right)
2. You see directories sorted by when you last watched something from them
3. Only directories with unwatched files are shown
4. Press `Enter` on a series to open the file browser
5. Browser shows unwatched files at the top, then alphabetically
6. Press `Enter` to add a file to the playlist

**From scratch:**
1. Press a key (TBD) to open the full file browser
2. Navigate your media root directories
3. Select files to add

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
   - Select the red entry, press a key (TBD) to open browser
   - Browser opens to the directory most recently used for files from that series
   - Files are sorted by edit distance to the target filename
4b. You can manually set yourself to "not watching" on a file that's Missing
   (e.g. a known series but you don't have this episode yet). This clears the
   "missing from known series" block
4c. You can select the file, then select 'download'. The file will be retrieved from
    peers using a bittorrent-like protocol.

Either of the three options can be made the default in the settings screen.

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

- **On join**: User State starts as Ready or Not Watching (depending on settings); File State depends on whether the file was found locally
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
6. Seeks are debounced (1000ms) — only broadcast after the user stops scrubbing

### Before Playback Starts

Before unpausing is allowed, DessPlay verifies file contents match:

1. Check file size
2. If match: Read 1MB chunks at 50MB offsets (through the entire file)
2. Combine with file size to create a hash
3. Compare hashes across all Ready users
4. If mismatch: unpause is blocked, File State is set to Missing

This prevents sync issues from different encodes/versions.

### Chat

- Type in the chat input (always visible at bottom of chat pane)
- Press Enter to send
- Messages appear in the chat pane AND as OSD in the video player
- System messages (joins, disconnects, state changes) appear in chat
- Text commands start with `/`:
  - `/exit`, `/quit`, `/q` — quit DessPlay

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

### Marking "Not Watching"

If you've already seen an episode or need to step away:

1. Focus the Playlist pane
2. Select the episode
3. Press a key (TBD) to toggle "not watching"
4. Your state changes to Not Watching (gray)
5. Playback can proceed without you
6. When the next file starts, you're automatically set back to normal (unless that file is also marked)

### Settings

Access settings screen to configure:

- Username
- Player preference (mpv/vlc)
- Media root directories (add/remove/reorder)

---

## TUI Layout

```
+----------------------------------+------------------+
|                                  | Recent Series    |
|                                  | (unwatched only, |
|          Chat Window             |  sorted by       |
|                                  |  recency)        |
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
- Right 50%, top: Recent Series
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
| `Tab` | Any | Cycle focus: Chat → Recent Series → Playlist → Chat |
| `Enter` | Chat | Send message (or execute `/command`) |
| `Esc` | Chat | Clear input |
| `Backspace` | Chat | Delete character before cursor |
| `Delete` | Chat | Delete character after cursor |
| `Left` / `Right` | Chat | Move cursor |
| `Ctrl-Left` / `Ctrl-Right` | Chat | Move cursor by word |
| `Home` / `End` | Chat | Move cursor to start/end |

Note: there is no `q` to quit — too easy to hit while typing in chat.

---

## Network Protocol

### Overview

- **Transport**: QUIC for all peer-to-peer communication
- **Topology**: Full mesh (everyone connects to everyone)
- **Sync model**: State-based, not event-based
- **Conflict resolution**: Per datatype, last-write-wins in most cases

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

The rendezvous server does NOT participate in state sync. Once peers are connected, they communicate directly.

### Time Synchronization

NTP-like protocol to establish shared clock:

1. Client sends timestamp `t1`
2. Server responds with `t1`, server time `t2`, response time `t3`
3. Client receives at `t4`
4. Calculate offset and round-trip time
5. Repeat periodically to maintain sync

All state timestamps use this shared clock.

### State Sync Protocol

All datagrams begin with a version byte (currently `1`). Peers that receive
an unknown version drop the message and log a warning.

Each peer periodically broadcasts full state:

```
StateSnapshot {
    version: u8,
    timestamp: SharedClockTime,
    playlist: Vec<PlaylistItem>,   // items have stable ItemIds
    current_file: Option<ItemId>,
    position: PositionRegister,    // playback position (LWW)
    user_states: HashMap<UserId, UserState>,
    file_states: HashMap<UserId, FileState>,
}
```

Playback state (playing vs paused) is **derived**, not synced directly:
the video plays iff every user's User State is Ready or Not Watching, and
their File State permits playback (Ready, or Downloading with sufficient
progress). This keeps position and readiness as independent concerns —
a seek cannot silently overwrite a pause, since they are in different fields.

On receiving a StateSnapshot, each field is merged independently:
- LWW registers: highest timestamp wins
- Append logs (playlist, chat): see network-design.md for details

Broadcast interval: 100ms during playback, 1s when paused

### Chat Protocol

Chat messages include:
- Sender username
- Message text
- Timestamp
- Sequence number (per-user)

Reliability: Chat uses the per-user append log mechanism (see `network-design.md`).
New messages are eager-pushed via datagrams; state vectors in periodic snapshots
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

Stored in local SQLite database. Configured via chat commands:
- `/add-root <path>` — add a media root directory
- `/remove-root <path>` — remove a media root directory
- `/list-roots` — show configured roots

Media roots are also shown at the top of the Recent Series pane for browsing.

### File Matching

When a playlist item is added:

1. Extract base filename (e.g., "Frieren - 01.mkv")
2. Search all media roots recursively for exact filename match
3. If found: store local path
4. If not found: mark as missing (red in UI)

### Manual File Mapping

When automatic matching fails:

1. User selects the missing item in playlist
2. Opens file browser
3. Browser starts in directory most recently used for files from the same source directory
4. Files sorted by edit distance to target filename
5. User selects correct file
6. Mapping stored locally

### Content Hash

Before playback can unpause:

1. Read file size
2. Read 1MB chunks at fixed 50MB offsets (through the entire file)
3. Combine with file size to create a hash
4. Compare with other Ready users

If hash mismatch: File State is set to Missing, cannot participate until resolved.

This is skipped for manually-mapped files (user explicitly chose a different file).

### Watch Tracking

- A file is "watched" when 90% of its duration has been played
- Tracked per-file in local database
- Used for:
  - Sorting "Recent Series" (most recently watched on top)
  - Filtering "unwatched files" in series browser
  - **Known series detection**: a series (source directory) is "known" if you
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

Uses `rusqlite` with `bundled` feature. All sync state is persisted per-room so it
survives full disconnects. On startup, the stored state is loaded and passed to the
sync engine as initial state.

### Schema (implemented)

```sql
-- Per-room chat messages (sync engine append log)
CREATE TABLE chat_messages (
    room TEXT NOT NULL,
    user_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    text TEXT NOT NULL,
    timestamp INTEGER NOT NULL,
    PRIMARY KEY (room, user_id, seq)
);

-- Per-room playlist entries (sync engine append log)
CREATE TABLE playlist_entries (
    room TEXT NOT NULL,
    user_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    action_json TEXT NOT NULL,  -- serde_json serialized PlaylistAction
    timestamp INTEGER NOT NULL,
    PRIMARY KEY (room, user_id, seq)
);

-- Per-room player state (LWW register)
CREATE TABLE player_state (
    room TEXT PRIMARY KEY,
    file_index INTEGER,
    position REAL NOT NULL DEFAULT 0.0,
    is_playing INTEGER NOT NULL DEFAULT 0,
    timestamp INTEGER NOT NULL DEFAULT 0
);

-- User settings (key-value)
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

All inserts use `INSERT OR IGNORE` for idempotency (gap fills may re-deliver entries).
State vectors and local sequence counters are derived from the stored logs on load.
Ready states are ephemeral and not persisted.

### Schema (implemented — file management)

```sql
-- Media root directories
CREATE TABLE media_roots (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    position INTEGER NOT NULL  -- for ordering
);

-- File watch history (also used for "known series" detection)
CREATE TABLE watch_history (
    filename TEXT PRIMARY KEY,
    directory TEXT NOT NULL,  -- parent directory, for known-series lookups
    last_watched INTEGER,
    watch_count INTEGER NOT NULL DEFAULT 0,
    last_position REAL NOT NULL DEFAULT 0.0,
    completed INTEGER NOT NULL DEFAULT 0  -- watched >= 90%
);

-- User settings (key-value)
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

### Schema (planned, not yet implemented)

```sql
-- Session markers
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    started TIMESTAMP,
    ended TIMESTAMP
);

-- Manual file mappings
CREATE TABLE file_mappings (
    playlist_filename TEXT,
    local_path TEXT,
    PRIMARY KEY (playlist_filename)
);
```

---

## Architecture

### Component Overview

```
+------------------+     +------------------+
|    TUI Layer     |     |  Player Bridge   |
|  (ratatui)       |<--->|  (mpv/vlc IPC)   |
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

### Crate Structure (tentative)

```
dessplay/
├── src/
│   ├── main.rs           # Entry point, CLI args
│   ├── tui/              # Terminal UI
│   │   ├── mod.rs
│   │   ├── app.rs        # Main app state
│   │   ├── widgets/      # Custom widgets
│   │   └── screens/      # Settings, main, browser
│   ├── core/             # Core state & logic
│   │   ├── mod.rs
│   │   ├── playlist.rs
│   │   ├── state.rs      # Shared state, sync
│   │   └── ready.rs      # Ready state machine
│   ├── network/          # Networking
│   │   ├── mod.rs
│   │   ├── rendezvous.rs
│   │   ├── peer.rs       # Peer connections
│   │   ├── stun.rs
│   │   ├── turn.rs
│   │   └── sync.rs       # State sync protocol
│   ├── player/           # Player integration
│   │   ├── mod.rs
│   │   ├── mpv.rs
│   │   ├── vlc.rs
│   │   └── bridge.rs     # Common interface
│   ├── storage/          # SQLite
│   │   ├── mod.rs
│   │   └── schema.rs
│   └── files/            # File management
│       ├── mod.rs
│       ├── search.rs
│       ├── hash.rs
│       └── browser.rs
├── src/bin/
│   ├── rendezvous.rs    # Rendezvous server binary
│   └── net_logger.rs    # Logging test client
└── tests/               # Integration tests
```

### Key Dependencies (tentative)

- **TUI**: `ratatui`, `crossterm`
- **Async**: `tokio`
- **Network**: `tokio`, `quinn` (QUIC)
- **Database**: `rusqlite`
- **Serialization**: `serde`, `postcard`
- **Player IPC**: Custom (mpv JSON-IPC, VLC Lua TCP)

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

## Open Questions

1. ~~**Keyboard shortcuts**~~: Resolved — see [Keyboard Shortcuts](#keyboard-shortcuts)
2. ~~**TURN implementation**~~: Resolved — minimal custom implementation over the rendezvous server
3. **Edit distance algorithm**: Levenshtein? Jaro-Winkler?
4. **Hash parameters**: How many bytes from how many offsets?
5. **Playlist compaction**: Design snapshot-based compaction for append logs (TODO — not needed for v1 but should be designed early)

---

## Future Plans

Besides those already mentioned:
- A subtitle pane, showing recent subtitles from the video. Optionally fused with the chat pane.
- Automated, sharded download from peers for missing files.
