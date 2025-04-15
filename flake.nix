# Build, testing, and developments specification for the `nix` environment
#
# # Prerequisites
#
# - Install the `nix` package manager: https://nixos.org/download/
# - Configure `flake` support: https://nixos.wiki/wiki/Flakes
#
# # Build
#
# ```
# $ nix build --print-build-logs
# ```
#
# This produces:
#
# - ./result/bin/zebra-scanner
# - ./result/bin/zebrad-for-scanner
# - ./result/bin/zebrad
# - ./result/book/
#
# The book directory is the root of the book source, so to view the rendered book:
#
# ```
# $ xdg-open ./result/book/book/index.html
# ```
#
# # Development
#
# ```
# $ nix develop
# ```
#
# This starts a new subshell with a development environment, such as
# `cargo`, `clang`, `protoc`, etc... So `cargo test` for example should
# work.
{
  description = "The zebra zcash node binaries and crates";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane.url = "github:ipetkov/crane";

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.rust-analyzer-src.follows = "";
    };

    flake-utils.url = "github:numtide/flake-utils";

    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, crane, fenix, flake-utils, advisory-db, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        inherit (pkgs) lib;

        # Print out a JSON serialization of the argument as a stderr diagnostic:
        enableTrace = false;
        traceJson =
          if enableTrace
          then (lib.debug.traceValFn builtins.toJSON)
          else (x: x);

        craneLib = crane.mkLib pkgs;

        # We use the latest nixpkgs `libclang`:
        inherit (pkgs.llvmPackages)
          libclang
        ;

        src = lib.sources.cleanSource ./.;

        # Common arguments can be set here to avoid repeating them later
        commonArgs = {
          inherit src;

          strictDeps = true;
          # NB: we disable tests since we'll run them all via cargo-nextest
          doCheck = false;

          # Use the clang stdenv:
          inherit (pkgs.llvmPackages) stdenv;

          nativeBuildInputs = with pkgs; [
            pkg-config
            protobuf
          ];

          buildInputs = with pkgs; [
            libclang
            rocksdb
          ];

          # Additional environment variables can be set directly
          LIBCLANG_PATH="${libclang.lib}/lib";
        };

        # Build *just* the cargo dependencies (of the entire workspace),
        # so we can reuse all of that work (e.g. via cachix) when running in CI
        cargoArtifacts = craneLib.buildDepsOnly (commonArgs // {
          pname = "zebrad-workspace-dependency-artifacts";
          version = "0.0.0";
        });

        individualCrateArgs = (crate:
          let
            result = commonArgs // {
              inherit cargoArtifacts;
              inherit (traceJson (craneLib.crateNameFromCargoToml { cargoToml = traceJson (crate + "/Cargo.toml"); }))
                pname
                version
              ;

              # BUG 1: We should not need this on the assumption that crane already knows the package from pname?
              # BUG 2: crate is a path, not a string.
              # cargoExtraArgs = "-p ${crate}";
            };
          in
            assert builtins.isPath crate;
            traceJson result
        );

        # Build the top-level crates of the workspace as individual derivations.
        # This allows consumers to only depend on (and build) only what they need.
        # Though it is possible to build the entire workspace as a single derivation,
        # so this is left up to you on how to organize things
        #
        # Note that the cargo workspace must define `workspace.members` using wildcards,
        # otherwise, omitting a crate (like we do below) will result in errors since
        # cargo won't be able to find the sources for all members.
        zebrad = craneLib.buildPackage (individualCrateArgs ./zebrad);

        zebra-book = pkgs.stdenv.mkDerivation rec {
          name = "zebra-book";
          src = ./.; # Note: The book refers to parent directories, so we need full source
          buildInputs = with pkgs; [
            mdbook
            mdbook-mermaid
          ];
          builder = pkgs.writeShellScript "${name}-builder.sh" ''
            source "$stdenv/setup"

            set -x

            cp -r "$src/" ./build
            cd ./build/book

            # Create the rendering output dir:
            chmod u+w .
            mkdir ./book
            chmod u-w .

            # Render:
            mdbook build

            # Copy to output:
            mkdir -p "$out/book"
            cp -r . "$out/book/"

            set +x
          '';
        };

        zebra-all-pkgs = pkgs.symlinkJoin {
          name = "zebra-all-pkgs";
          paths = [
            zebrad
            zebra-book
          ];
        };
      in
      {
        checks = {
          # Build the crates as part of `nix flake check` for convenience
          inherit zebrad;

          # Run clippy (and deny all warnings) on the workspace source,
          # again, reusing the dependency artifacts from above.
          #
          # Note that this is done as a separate derivation so that
          # we can block the CI if there are issues here, but not
          # prevent downstream consumers from building our crate by itself.

          # my-workspace-clippy = craneLib.cargoClippy (commonArgs // {
          #   inherit (zebrad) pname version;
          #   inherit cargoArtifacts;

          #   cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          # });

          my-workspace-doc = craneLib.cargoDoc (commonArgs // {
            inherit (zebrad) pname version;
            inherit cargoArtifacts;
          });

          # Check formatting
          my-workspace-fmt = craneLib.cargoFmt {
            inherit (zebrad) pname version;
            inherit src;
          };

          # my-workspace-toml-fmt = craneLib.taploFmt {
          #   src = pkgs.lib.sources.sourceFilesBySuffices src [ ".toml" ];
          #   # taplo arguments can be further customized below as needed
          #   # taploExtraArgs = "--config ./taplo.toml";
          # };

          # Audit dependencies
          my-workspace-audit = craneLib.cargoAudit {
            inherit (zebrad) pname version;
            inherit src advisory-db;
          };

          # Audit licenses
          # my-workspace-deny = craneLib.cargoDeny {
          #   inherit (zebrad) pname version;
          #   inherit src;
          # };

          # Run tests with cargo-nextest
          # Consider setting `doCheck = false` on other crate derivations
          # if you do not want the tests to run twice
          my-workspace-nextest = craneLib.cargoNextest (commonArgs // {
            inherit (zebrad) pname version;
            inherit cargoArtifacts;

            partitions = 1;
            partitionType = "count";
          });
        };

        packages = {
          inherit zebrad;

          default = zebra-all-pkgs;
        };

        apps = {
          zebrad = flake-utils.lib.mkApp {
            drv = zebrad;
          };
        };

        devShells.default = (
          let
            mkClangShell = pkgs.mkShell.override {
              inherit (pkgs.llvmPackages) stdenv;
            };

            devShellInputs = with pkgs; [
              rustup
              mdbook
              mdbook-mermaid
            ];

          in mkClangShell (commonArgs // {
            # Include devShell inputs:
            nativeBuildInputs = commonArgs.nativeBuildInputs ++ devShellInputs;
          })
        );
      });
}

