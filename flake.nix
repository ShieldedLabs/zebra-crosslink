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

          # Use the version out of `zebrad`'s package metadata.
          version = (
            let
              inherit (builtins) readFile fromTOML;
              cargoToml = fromTOML (readFile ./zebrad/Cargo.toml);
            in
              cargoToml.package.version
          );

          # We use standard nixpkgs-unstable:
          overlays = [];
          pkgs = import nixpkgs {
            inherit system overlays;
          };

          # Import nix-specific plumbing:
          inherit (pkgs)
            mkShell
            writeScript
          ;

          # We use the latest `libclang`:
          inherit (pkgs.llvmPackages)
            libclang
          ;

          # Native (non-cargo) dependencies which only need to be present during builds:
          nativeBuildInputs = with pkgs; [
            pkg-config
            protobuf
          ];

          # Native (non-cargo) dependencies which need to be present during build- & run- times:
          buildInputs = with pkgs; [
            libclang
            rocksdb
          ];

          # In `nix develop` mode, we have to explicitly make `cargo` available (in `nix build` time the `rustPlatform` nix API manages rust builds):
          devShellInputs = with pkgs; [
            cargo
          ];
        in {
          packages.default = abort "`nix build` not yet implemented";

          # The `nix develop` shell:
          # - Uses `clang` for C compiler.
          # - Includes `cargo` plus all `nativeBuildOutputs`.
          # - Sets `LIBCLANG_PATH` explicitly.
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
