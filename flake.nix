{
  description = "silent-pool: SPc Stratum pool (SV1 + SV2), SLICE accounting, Ark tier";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        rust = (pkgs.rust-bin.stable."1.93.0".default.override {
          extensions = [ "rust-src" "rustfmt" "clippy" ];
        });
        src = pkgs.lib.cleanSource ./.;
        common = {
          inherit src;
          rustc = rust;
          cargo = rust;
          nativeBuildInputs = [ rust pkgs.pkg-config ];
          buildInputs = [ pkgs.openssl pkgs.sqlite ];
          cargoLock.lockFile = ./Cargo.lock;
          doCheck = false;
        };
        mkBinary = name: pkgs.rustPlatform.buildRustPackage (common // {
          pname = name;
          version = "0.1.0";
          cargoBuildFlags = [ "-p" name ];
          meta.mainProgram = name;
        });
        mkCargoCheck = name: buildPhase: pkgs.rustPlatform.buildRustPackage (common // {
          pname = name;
          version = "0.1.0";
          inherit buildPhase;
          installPhase = "touch $out";
        });
        rustEnv = {
          RUST_BACKTRACE = "1";
          SPC_TEST_MODE = "1";
        };
      in {
        packages = {
          spc-pool = mkBinary "spc-pool";
          spc-agent = mkBinary "spc-agent";
          spc-scan = mkBinary "spc-scan";
          default = mkBinary "spc-pool";
        };

        checks = {
          fmt = mkCargoCheck "spc-fmt" "cargo fmt --all -- --check";

          clippy = mkCargoCheck "spc-clippy"
            "cargo clippy --workspace --all-targets --offline -- -D warnings";

          doc = mkCargoCheck "spc-doc"
            "cargo doc --workspace --no-deps --offline";

          test = pkgs.rustPlatform.buildRustPackage (common // {
            pname = "spc-workspace-tests";
            version = "0.1.0";
            doCheck = true;
            checkPhase = ''
              runHook preCheck
              cargo test --workspace --offline
              runHook postCheck
            '';
          });
        };

        devShells.default = pkgs.mkShell ({
          packages = [
            rust
            pkgs.cargo-nextest
            pkgs.cargo-audit
            pkgs.cargo-deny
            pkgs.just
            pkgs.pkg-config
            pkgs.openssl
            pkgs.sqlite
            pkgs.postgresql_17
            pkgs.redis
            pkgs.bitcoind
            pkgs.jq
            pkgs.curl
            pkgs.sqlx-cli
            pkgs.git
          ];
          shellHook = ''
            echo "silent-pool devshell"
            echo "  just check        fmt + clippy + tests"
            echo "  just e2e          regtest scenarios"
            echo "  nix flake check   what CI runs"
          '';
        } // rustEnv);
      }) // {
        nixosModules = {
          spc-pool = import ./nix/modules/spc-pool.nix { inherit self; };
          spc-agent = import ./nix/modules/spc-agent.nix { inherit self; };
          spc-stack = import ./nix/modules/spc-stack.nix { inherit self; };
          default = self.nixosModules.spc-stack;
        };

        overlays.default = final: prev: {
          spc-pool = self.packages.${final.system}.spc-pool;
          spc-agent = self.packages.${final.system}.spc-agent;
          spc-scan = self.packages.${final.system}.spc-scan;
        };
      };
}
