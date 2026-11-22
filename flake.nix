{
  description = "vo1ded-panel";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
  };

  outputs = {
    self,
    nixpkgs,
  }: let
    system = "x86_64-linux";
    pkgs = nixpkgs.legacyPackages.${system};

    nativeBuildInputs = with pkgs; [
      pkg-config
      dart-sass
      glib # glib-compile-resources
      rustPlatform.bindgenHook
      wrapGAppsHook4
    ];

    buildInputs = with pkgs; [
      gtk4
      gtk4-layer-shell
      pipewire
      glib
    ];

    # One package per binary of the crate.
    mkPackage = bin:
      pkgs.rustPlatform.buildRustPackage {
        pname = bin;
        inherit nativeBuildInputs buildInputs;
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        cargoBuildFlags = ["--bin" bin];
        meta.mainProgram = bin;
      };
  in {
    packages.${system} = rec {
      vo1ded-panel = mkPackage "vo1ded-panel";
      vo1ded-dialog = mkPackage "vo1ded-dialog";
      default = vo1ded-panel;
    };

    devShells.${system}.default = pkgs.mkShell {
      inherit buildInputs;
      nativeBuildInputs =
        nativeBuildInputs
        ++ (with pkgs; [
          cargo
          rustc
          rust-analyzer
          clippy
          rustfmt
        ]);
    };
  };
}
