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

- **NixOS** — only `nix` is required. The launcher builds and runs inside a
  `nix-shell` that pulls the toolchain (`cargo`, `rustc`), `pkg-config`,
  `openssl`, `libwebp`, and `mpv` from your system `<nixpkgs>` channel. It does
  **not** use flakes, so flakes do not need to be enabled.
- **Other distros** (CachyOS/Arch, Debian/Ubuntu, Fedora, …) — you need:
  - `git`
  - a Rust toolchain (`cargo`; distro package or [rustup](https://rustup.rs))
  - `pkg-config`
  - the `openssl` and `libwebp` development headers
  - `mpv` (the default player)

  Whatever `cargo` you have is used (stable is fine).

## Notes

- The rendezvous server and seeder (`dessplay-rendezvous`) are deployed
  separately as systemd services and are **not** covered by this launcher.
- Uninstall by removing `~/.local/bin/dessplay` and `~/.cache/dessplay`.
