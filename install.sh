#!/usr/bin/env sh
#
# DessPlay bootstrap / launcher.
#
# This single script is BOTH the curl|sh installer and the installed launcher
# (`dessplay`); the installed command is just a symlink back to this file inside
# the clone, so there is never a second copy to drift.
#
#   Install:   curl -fsSL https://raw.githubusercontent.com/Baughn/dessplay/master/install.sh | sh
#   Run after: dessplay [args...]
#
# On first run (no clone yet) it clones the repo and symlinks itself into
# ~/.local/bin, then launches. On every later run it pulls the latest commit and
# builds/runs the main `dessplay` binary -- via nix-shell whenever Nix is
# installed (NixOS, or any distro / macOS with the Nix package manager), or via
# the system's own cargo otherwise.

set -eu

REPO_URL="https://github.com/Baughn/dessplay.git"
CACHE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/dessplay"
REPO_DIR="$CACHE_DIR/repo"
BIN_DIR="$HOME/.local/bin"
BIN_LINK="$BIN_DIR/dessplay"

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*" >&2; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

have() { command -v "$1" >/dev/null 2>&1; }

# Quote a single argument for safe reuse inside a `sh -c`/`--run` string.
quote() {
    printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"
}

# ---------------------------------------------------------------------------
# Distro / package-manager detection (for the "you're missing X" hints).
# ---------------------------------------------------------------------------

# True when we can build inside a nix-shell: any system with the Nix package
# manager installed -- NixOS, or an ordinary distro / macOS (incl. nix-darwin)
# where the user installed Nix. We prefer it whenever available: it pulls every
# build/runtime dependency (cargo, openssl, libwebp, mpv, ...) from nixpkgs, so
# nothing else need be on PATH. Detect the tool, not the OS -- a non-NixOS Nix
# install (e.g. nix-darwin on macOS) has no /etc/NIXOS marker but still has
# nix-shell.
have_nix() { have nix-shell; }

# Echo a `sudo <pm> ...` install command for the given package keywords,
# choosing package names per the detected distro family. Best-effort: if we
# can't tell, we fall back to a generic note.
install_hint() {
    _id=""; _like=""
    if [ -r /etc/os-release ]; then
        # shellcheck disable=SC1091
        ID=""; ID_LIKE=""; . /etc/os-release 2>/dev/null || true
        _id="${ID:-}"; _like="${ID_LIKE:-}"
    fi
    case " $_id $_like " in
        *" arch "*|*" cachyos "*|*" manjaro "*|*" endeavouros "*)
            echo "sudo pacman -S --needed git rust pkgconf openssl libwebp mpv" ;;
        *" debian "*|*" ubuntu "*|*" linuxmint "*|*" pop "*)
            echo "sudo apt install git cargo pkg-config libssl-dev libwebp-dev mpv" ;;
        *" fedora "*|*" rhel "*|*" centos "*)
            echo "sudo dnf install git cargo pkgconf-pkg-config openssl-devel libwebp-devel mpv" ;;
        *)
            echo "(install: git, a Rust toolchain/cargo, pkg-config, openssl + libwebp dev headers, mpv)" ;;
    esac
}

# ---------------------------------------------------------------------------
# Install mode: clone + symlink, then hand off to run mode.
# ---------------------------------------------------------------------------

install_mode() {
    have git || die "git is required to install dessplay. $(install_hint)"

    if [ ! -d "$REPO_DIR/.git" ]; then
        say "Cloning dessplay into $REPO_DIR"
        mkdir -p "$CACHE_DIR"
        git clone --depth 1 "$REPO_URL" "$REPO_DIR"
    fi

    mkdir -p "$BIN_DIR"
    ln -sf "$REPO_DIR/install.sh" "$BIN_LINK"
    say "Installed launcher at $BIN_LINK"

    case ":$PATH:" in
        *":$BIN_DIR:"*) ;;
        *) warn "$BIN_DIR is not on your PATH; add it to your shell rc, e.g.:
    export PATH=\"\$HOME/.local/bin:\$PATH\"" ;;
    esac

    say "Launching dessplay"
    exec "$REPO_DIR/install.sh" "$@"
}

# ---------------------------------------------------------------------------
# Run mode: pull, then build/run the main binary.
# ---------------------------------------------------------------------------

require_tools() {
    missing=""
    for t in git cargo pkg-config mpv; do
        have "$t" || missing="$missing $t"
    done
    if have pkg-config; then
        pkg-config --exists openssl 2>/dev/null || missing="$missing openssl-dev"
        pkg-config --exists libwebp 2>/dev/null || missing="$missing libwebp-dev"
    fi
    if [ -n "$missing" ]; then
        warn "missing build/runtime dependencies:$missing"
        say  "install them, then re-run dessplay:"
        printf '    %s\n' "$(install_hint)" >&2
        case "$missing" in
            *cargo*) printf '    (Rust may instead come from rustup: https://rustup.rs)\n' >&2 ;;
        esac
        exit 1
    fi
}

run_mode() {
    cd "$REPO_DIR" || die "clone missing at $REPO_DIR; re-run the installer"

    if have git; then
        say "Updating ($REPO_DIR)"
        git pull --ff-only || warn "git pull failed; running the existing checkout"
    fi

    if have_nix; then
        say "Nix detected; building inside nix-shell"
        # No flakes, no flake.lock: a plain shell expression over <nixpkgs>
        # (a channel, or Determinate Nix's built-in flake fallback), mirroring
        # flake.nix's build inputs.
        nix_expr='let pkgs = import <nixpkgs> {};
in pkgs.mkShell {
  buildInputs = [ pkgs.cargo pkgs.rustc pkgs.pkg-config pkgs.openssl pkgs.libwebp pkgs.mpv pkgs.git ];
  PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
}'
        run_cmd="cargo run --release -p dessplay --"
        for arg in "$@"; do
            run_cmd="$run_cmd $(quote "$arg")"
        done
        exec nix-shell -E "$nix_expr" --run "$run_cmd"
    else
        require_tools
        say "Building and launching dessplay"
        exec cargo run --release -p dessplay -- "$@"
    fi
}

# ---------------------------------------------------------------------------

if [ ! -f "$REPO_DIR/flake.nix" ]; then
    install_mode "$@"
fi
run_mode "$@"
