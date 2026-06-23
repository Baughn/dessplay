# DessPlay

A synchronized video player for watch parties. Terminal-first, built for
reliability over flaky connections. Server-coordinated, including relayed file
transfer between peers.

See [`docs/design.md`](docs/design.md) for the full design, and the rest of
[`docs/`](docs/) for architecture, sync state, networking, testing, and the UI.

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
