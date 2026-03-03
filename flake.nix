{
  description = "DessPlay — collaborative media playback";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
      buildRustPackage = pkgs.rustPlatform.buildRustPackage;
    in
    {
      packages.${system} = {
        rendezvous = buildRustPackage {
          pname = "dessplay-rendezvous";
          version = "0.1.0";
          src = ./.;
          cargoBuildFlags = [ "-p" "dessplay-rendezvous" ];
          cargoLock.lockFile = ./Cargo.lock;
        };

        default = self.packages.${system}.rendezvous;
      };
    };
}
