{
  description = "Dessplay";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, crane, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        lib = pkgs.lib;

        rustToolchain = pkgs.rust-bin.nightly.latest.default.override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Keep Cargo sources plus the assets that `include_str!` / `include_bytes!`
        # pull in at compile time (prompts, fonts).
        src = lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            (lib.hasSuffix ".ttf" path)
            || (craneLib.filterCargoSources path type);
          name = "dessplay-source";
        };

        commonArgs = {
          inherit src;
	  pname = "dessplay";
          strictDeps = true;

          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [ openssl libwebp ];

          PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
        };

        # Build *only* the dependencies. Cached until Cargo.lock changes.
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Build the workspace, reusing the pre-built dependency artifacts.
        dessplay = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
          doCheck = false;
        });

        devBuildInputs = with pkgs; [
          pkg-config
          openssl

          rustToolchain
          cargo-watch
          cargo-edit
          cargo-insta
          cargo-outdated
          cargo-audit
          cargo-machete
          cargo-features-manager
          cargo-flamegraph
	  samply
          bacon
	  python3
        ];
      in
      {
        packages.default = dessplay;

        # Dev shell keeps mold + clang for fast incremental local builds.
        devShells.default = pkgs.mkShell.override {
	  #stdenv = if stdenv.targetPlatform.isDarwin then pkgs.clangStdenv else pkgs.stdenvAdapters.useMoldLinker pkgs.clangStdenv;
        } {
          buildInputs = devBuildInputs;
          nativeBuildInputs = [ rustToolchain ];

          PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";
          RUST_BACKTRACE = 1;
          RUST_LOG = "dessplay=debug";

          shellHook = ''
            echo "Ganbot Rust development environment"
            echo "Rust version: $(rustc --version)"
            echo ""
            echo "Available commands:"
            echo "  cargo build    - Build the project"
            echo "  cargo run      - Run the bot"
            echo "  cargo watch    - Watch for changes and rebuild"
            echo "  cargo test     - Run tests"
            echo "  cargo check    - Check for compilation errors"
            echo "  bacon          - Run bacon for continuous checking"
            echo "  cargo machete   - Remove old deps"
            echo "  cargo features prune"
            echo "  eslint         - Lint JavaScript files"
            echo ""
          '';
        };

        apps.default = flake-utils.lib.mkApp {
          drv = dessplay;
        };

        # Useful extra checks you can run with `nix flake check`.
        checks = {
          inherit dessplay;

          dessplay-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- -D warnings";
          });

          dessplay-fmt = craneLib.cargoFmt {
            inherit src;
          };
        };
      });
}
