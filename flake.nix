{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs = { self, nixpkgs, flake-utils }: (
    flake-utils.lib.eachDefaultSystem (
      system: (
        let
          pname = "zebra-crosslink";
          version = (
            let
              inherit (builtins) readFile fromTOML;
              cargoToml = fromTOML (readFile ./zebrad/Cargo.toml);
            in
              cargoToml.package.version
          );

          overlays = [];
          pkgs = import nixpkgs {
            inherit system overlays;
          };

          inherit (pkgs)
            # nix plumbing:
            mkShell
            writeScript
          ;

          inherit (pkgs.llvmPackages)
            libclang
          ;

          nativeBuildInputs = with pkgs; [
            pkg-config
            protobuf
          ];

          buildInputs = with pkgs; [
            libclang
            rocksdb
          ];

          devShellInputs = with pkgs; [
            cargo
          ];
        in {
          # packages.default = pkgs.rustPlatform.buildRustPackage {
          #   inherit pname version nativeBuildInputs buildInputs;
          #   src = ./.;
          #   # cargoBuildFlags = "-p app";

          #   # patchPhase = ''

          #   #   set -x

          #   #   PKG_CONFIG_ALLOW_SYSTEM_LIBS=1 PKG_CONFIG_ALLOW_SYSTEM_CFLAGS=1 pkg-config --libs --cflags fontconfig

          #   #   set +x
          #   #   exit 1
          #   # '';

          #   cargoLock = {
          #     lockFile = ./Cargo.lock;
          #   };
          # };

          devShells.default = (mkShell.override { stdenv = pkgs.llvmPackages.stdenv; }) {
            inherit buildInputs;
            nativeBuildInputs = nativeBuildInputs ++ devShellInputs;

            LIBCLANG_PATH="${libclang.lib}/lib";
          };
        }
      )
    )
  );
}
