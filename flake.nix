{
  description = "DessPlay — synchronized media playback";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs, ... }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
      rendezvous = pkgs.rustPlatform.buildRustPackage {
        pname = "dessplay-rendezvous";
        version = "0.1.0";

        src = ./.;

        cargoLock.lockFile = ./Cargo.lock;

        cargoBuildFlags = [ "--bin" "dessplay-rendezvous" ];

        doCheck = false;
      };
    in
    {
      packages.${system} = {
        inherit rendezvous;
        default = rendezvous;
      };
    };
}
