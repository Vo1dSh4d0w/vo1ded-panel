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
    pname = "vo1ded-panel";

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
  in {
    packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
      inherit pname nativeBuildInputs buildInputs;
      version = "0.1.0";
      src = ./.;
      cargoLock.lockFile = ./Cargo.lock;
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
