# DessPlay

A synchronized video player for watch parties. Terminal-first, built for
reliability over flaky connections. Server-coordinated, including relayed file
transfer between peers.

See [`docs/design.md`](docs/design.md) for the full design, and the rest of
[`docs/`](docs/) for architecture, sync state, networking, testing, and the UI.

<!-- TODO: screenshot of the TUI mid-session goes here. -->

## Why not Syncplay?

DessPlay started as a replacement for [Syncplay](https://syncplay.pl/)
for a group of five people who watch a season of something together every
week. Syncplay solves "keep strangers' arbitrary players in step on a
public server." DessPlay solves "nobody should have to touch a file
manager before the episode starts." Those are different problems, and the
table below is what happens when you build for the second one and then
don't stop.

| Area | Syncplay | DessPlay |
|---|---|---|
| Sync core | Server-held position; slowdown, rewind, and fast-forward with three thresholds; seek undo; manual offset | Three-band drift control with hysteresis and rate-limited pitch-corrected slew; seek authority register; leader election validated by file identity; resume from the furthest position anyone reached |
| State model | Ad hoc messages; server keeps state in RAM | CRDTs with epochs, daily compaction, persisted snapshots, tagged migrations, and a `/resync` escape hatch |
| Readiness | Ready checkbox, autoplay at N ready, pause on leave | Derived from per-series commitment (Watching / Maybe / NotWatching), a manual override, Away-by-proxy, per-file acknowledgement of absent committed users, and offline users who still gate for a week |
| File identity | Filename, size, and duration mismatch warnings | ed2k content hash; mid-write mismatch re-check; manual mapping; drag-into-mpv adoption |
| Getting the file | None ("explicitly not a file-sharing service") | Relayed peer transfer over QUIC with BBR-style flow control and DSCP tagging; playback from a partial file behind a 20% window; prefetch anchored at now-playing; a headless seeder role; hash-addressed download cache with retention and archiving; nyaa search through an embedded BitTorrent engine |
| Playlist | Shared, shuffle, undo, URL streams, trusted domains, loop | Shared CRDT playlist with play history, watched flags, a franchise browser, and an episode browser that disambiguates between copies |
| Metadata | None | AniDB integration on the server (rate-limit ladder, relations graph, titles-dump search, AI-curated short titles) |
| Series tracking | None | The List: statuses, watchers, next-episode auto-advance, CSV import, and identity for series AniDB has never heard of |
| Chat | Text chat; OSD in mpv with chat input inside mpv | Chat, OSD overlays, mention highlighting, tab completion, `\|\|spoiler\|\|` scrambling with click-to-reveal, `/me`, drag-select to clipboard, derived narrator lines, day separators at 09:00 |
| External chat | None | IRC bridge with nick sanitising, spoiler masking, and `/summon` |
| Subtitles | None | Local subtitle log, intermixed or in its own pane, with a perceptually-spaced speaker palette |
| Players | mpv, mpv.net, VLC, MPC-HC, MPC-BE, IINA, memento, mplayer2 | mpv (VLC is a settings placeholder); crash ladder; attach mode for ssh |
| Multi-tenancy | Rooms, managed rooms with operators, room isolation, MOTD | One implicit room, one shared password |
| UI | Qt GUI plus a console mode | TUI only; true-color theme, mouse support, resizable panes, in-app changelog |
| Localisation | 13 languages | English |
| Platforms | Windows, Linux, BSD, macOS; packaged installers | Linux and macOS; built from source on every launch |
| Security | Optional TLS; hashed filenames for privacy on public servers | Always-on TLS with TOFU pinning; threat model is "there are five of us" |
| Gimmicks | None | An in-character AI commentary marquee that reacts to the episode |

For scale, as of September 2026:

| | Syncplay | DessPlay |
|---|---|---|
| Age | 14 years | 6.5 months, 63 days with commits |
| Contributors | 101 | 1, plus Claude |
| Core code | roughly 15 to 20k lines of Python and Lua | roughly 89k lines of Rust |
| Tests | none in the repository | roughly 1,370 test functions, 12 fuzz targets, 27 property-test suites |

Syncplay is missing nothing it set out to have. DessPlay is missing a
second player, a GUI, and any reason for a stranger to run it.


## Install

```sh
curl -fsSL https://raw.githubusercontent.com/Baughn/dessplay/master/install.sh | sh
```

This installs the **interactive `dessplay` client**. The one-liner:

1. Clones the repo into `~/.cache/dessplay/repo`.
2. Symlinks a `dessplay` launcher into `~/.local/bin` (add that to your `PATH`
   if it isn't already).
3. Launches DessPlay.

Afterwards just run:

```sh
dessplay
```

Every launch does a `git pull --ff-only` and rebuilds, so you always run the
latest commit. (If the pull fails — e.g. you're offline — it runs the existing
checkout instead.)

## Requirements

The launcher checks for what it needs and, if something is missing, prints the
exact command to install it. It never installs system packages for you.

- **Nix** (NixOS, or any distro / macOS with the Nix package manager) — only
  `nix` is required. The launcher builds and runs inside a `nix-shell` that
  pulls the toolchain (`cargo`, `rustc`), `pkg-config`, `openssl`, `libwebp`,
  and `mpv` from `<nixpkgs>` (your channel, or Determinate Nix's built-in flake
  fallback). It does **not** use flakes, so flakes do not need to be enabled.
- **Other systems without Nix** (CachyOS/Arch, Debian/Ubuntu, Fedora, …) — you need:
  - `git`
  - a Rust toolchain (`cargo`; distro package or [rustup](https://rustup.rs))
  - `pkg-config`
  - the `openssl` and `libwebp` development headers
  - `mpv` (the default player)

  Whatever `cargo` you have is used (stable is fine).

## Developing without a desktop

Normally DessPlay spawns its own mpv window, which needs a display. To work on
it remotely (e.g. over ssh) you can instead point DessPlay at an mpv you launch
yourself with `--attach-mpv=<socket>`.

In one terminal — ideally a separate tmux pane — launch mpv with terminal video
output (`--vo=tct`) and a JSON IPC socket:

```sh
mpv --idle=yes --keep-open=yes --vo=tct --input-ipc-server=/tmp/dessplay-mpv.sock
```

In another, attach DessPlay to it:

```sh
dessplay --attach-mpv=/tmp/dessplay-mpv.sock
```

mpv accepts multiple IPC clients at once, so DessPlay drives loads, seeks, and
playback while your keyboard in the mpv pane still pauses (space) and scrubs
(arrows) directly — a manual pause there propagates to the group exactly like
one in a normal window. The `--idle --keep-open` flags are required (DessPlay's
EOF and load handling depend on them). DessPlay leaves your mpv running when it
exits, and re-attaches if you restart it.

The picture is low-fidelity (`tct` renders video as colored terminal cells), so
this is a development aid, not a way to actually watch anything. If your
terminal supports them, `--vo=sixel` or `--vo=kitty` look considerably better.

## Notes

- The rendezvous server and seeder (`dessplay-rendezvous`) are deployed
  separately as systemd services and are **not** covered by this launcher.
- Uninstall by removing `~/.local/bin/dessplay` and `~/.cache/dessplay`.
