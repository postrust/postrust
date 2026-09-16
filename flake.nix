{
  description = "Postrust – PostgREST-compatible REST and GraphQL API server";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        # Build the postrust binary, mirroring what Dockerfile.alpine does:
        #   cargo build --release -p postrust-server --features admin-ui
        #
        # Native build inputs match the Alpine apk packages:
        #   musl-dev / build-base  → provided by the Nix C toolchain (stdenv)
        #   cmake                  → cmake
        #   perl                   → perl
        # aws-lc-sys also needs pkg-config at build time.
        postrust = pkgs.rustPlatform.buildRustPackage {
          pname = "postrust";
          version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

          src = ./.;

          # Uses the Cargo.lock in the repo directly; no separate hash needed.
          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          # Build only the server binary, with the same feature set as the Dockerfile.
          cargoBuildFlags = [
            "--package" "postrust-server"
            "--features" "admin-ui"
          ];
          # Skip tests during `nix build` (run them separately with `nix flake check`).
          doCheck = false;

          nativeBuildInputs = with pkgs; [
            cmake    # required by aws-lc-sys
            perl     # required by aws-lc-sys
            pkg-config
          ];

          meta = with pkgs.lib; {
            description = "PostgREST-compatible REST and GraphQL API server for PostgreSQL";
            homepage = "https://postrust.org/";
            license = licenses.mit;
            maintainers = [];
            mainProgram = "postrust";
          };
        };
      in
      {
        packages = {
          inherit postrust;
          default = postrust;
        };

        # `nix run` support
        apps.default = flake-utils.lib.mkApp {
          drv = postrust;
          name = "postrust";
        };

        # Development shell with the same toolchain + build deps
        devShells.default = pkgs.mkShell {
          inputsFrom = [ postrust ];
          buildInputs = with pkgs; [
            rustc
            cargo
            clippy
            rustfmt
          ];
        };
      });
}
