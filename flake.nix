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
        # Local LP-3 worktree has new untracked crates; cleanSource (git)
        # excludes them. Keep the source filter explicit so the merge session
        # can restore cleanSource after the crates are tracked.
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            let
              base = baseNameOf (toString path);
              relative = pkgs.lib.removePrefix (toString ./. + "/") (toString path);
            in !(builtins.elem base [ "target" ".git" ".direnv" ".cache" "__pycache__" "node_modules" ]
              || builtins.elem relative [ "spc_ark_client/test" "spc_ark_e2e/test" ]
              || pkgs.lib.hasPrefix "result" base);
        };
        common = {
          inherit src;
          rustc = rust;
          cargo = rust;
          # Same-source binding for the wallet-native-joint check: set from
          # the flake itself, never a shell variable. A clean git checkout
          # claims its commit; a dirty-tree snapshot stays "nix-snapshot" and
          # carries its true NAR/path binding instead of a commit claim.
          SPC_BUILD_SOURCE_ID = if self ? rev then "git:${self.rev}" else "nix-snapshot";
          # bark-server-rpc's build script generates protobuf code and
          # needs a `protoc` on PATH.
          PROTOC = "${pkgs.protobuf}/bin/protoc";
          nativeBuildInputs = [ rust pkgs.pkg-config pkgs.protobuf ];
          buildInputs = [ pkgs.openssl pkgs.sqlite ];
          # The workspace pins git deps (bark at c419159, decision
          # 2026-11-02-bark-adapter.md), which `cargoLock.lockFile`
          # cannot vendor; fetchCargoVendor runs `cargo vendor` on the
          # source and covers registry and git sources alike.
          cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
            inherit src;
            # 2026-11-11: refreshed after the wallet-service integration
            # added bdk_bitcoind_rpc + rustix to Cargo.lock (the previous
            # pin sha256-P3Gf1XW2CWkAO2IT6M3y37sssZsoh9Hu4osyLWd0K+E=
            # predates that lock delta and fails the Cargo.lock
            # consistency check).
            # 2026-11-12: refreshed again after the fullrail e5283a85 union
            # intake (spc-accounting ladder, pool ark wallet-control,
            # store/aggregate join seams) changed the spc-cpu-miner /
            # spc-lab / spc-agent dependency sets in Cargo.lock (previous
            # pin sha256-y/pymSKJfdDGBLdzUwMbVu5+zqGIKt03IZxVZ4L61hA=).
            hash = "sha256-Cij0Q0TwtwxODWxinPiH2kXi9DznDF4yMqMAACyDjQI=";
          };
          doCheck = false;
        };
        mkBinary = name: features: pkgs.rustPlatform.buildRustPackage (common // {
          pname = name;
          version = "0.1.0";
          cargoBuildFlags = [ "-p" name ]
            ++ pkgs.lib.optional (features != "") [ "--features" features ];
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
        arkPackages = import ./nix/lab/ark.nix { inherit pkgs rust; };
      in {
        packages = rec {
          rbitcoin-node-ark-lab = import ./nix/lab/ark-node.nix { inherit pkgs; };
          spc-lab-self-hosted-ark = import ./nix/lab/ark-runtime.nix {
            inherit pkgs src;
            lab = spc-lab-ark-tools;
            node = rbitcoin-node-ark-lab;
            pool = spc-lab-ark-tools;
            miner = spc-lab-ark-tools;
            scanner = spc-lab-ark-tools;
            agent = spc-lab-ark-tools;
            txd = spc-lab-ark-tools;
            probe = import ./nix/lab/ark-probe.nix { inherit pkgs; };
            inherit (arkPackages) captaind;
          };
          # One compilation for the local infrastructure launch closure, rather
          # than four independent copies of the same dependency build tree.
          spc-lab-ark-tools = pkgs.rustPlatform.buildRustPackage (common // {
            pname = "spc-lab-ark-tools";
            version = "0.1.0";
            postInstall = ''
              test -x "$out/bin/pool-wallet-executor"
            '';
            cargoBuildFlags = [ "-p" "spc-lab-cli" "-p" "spc-pool"
              "-p" "spc-cpu-miner" "-p" "spc-scan" "-p" "spc-agent"
              "-p" "spc-lab-txd"
              "--features" "spc-pool/rpc,spc-pool/ark,spc-agent/ark" ];
            doCheck = true;
            checkPhase = ''
              runHook preCheck
              cargo test --release --offline -p spc-lab \
                --test ark_infrastructure --test lifecycle_stop \
                --test two_runs_isolation --test supervisor_adoption \
                --test operator_ui_inventory
              runHook postCheck
            '';
            meta.mainProgram = "spc-lab";
          });
          inherit (arkPackages) captaind bark;
          # Portable AP5 harness/verifier/qualification set (packet
          # AP5-portable-integration-r1). The qualified derivation reuses the
          # joined spc-lab-ark-tools build (same dependency/target layout) and
          # only adds the source-contract preBuild check and the gate-plan
          # installCheck; it is a second OUTPUT of the same build inputs, not a
          # second application build. Until the real source-contract.json is
          # committed (state source-bound-not-tested), evaluating
          # spc-lab-ap5-qualified/ap5-inputs fails the contract assertion by
          # design - the shipped template cannot qualify.
          ap5Packages = import ./testing/lab-ark-funded/nix/packages.nix {
            inherit pkgs rust src;
            revision = if self ? rev then self.rev else "nix-snapshot";
            labTools = spc-lab-ark-tools;
            node = rbitcoin-node-ark-lab;
            inherit (arkPackages) captaind bark;
            probe = import ./nix/lab/ark-probe.nix { inherit pkgs; };
          };
          inherit (ap5Packages) ap5-tools ap5-vtxo-verifier ap5-qa-source
            ap5-inputs;
          # The qualified derivation's install-check gate executes the frozen
          # reviewed test sources (R4 pool-control wallet_control tests,
          # cpu-miner evidence/lifecycle helpers, agent native-source-audit
          # helpers) whose scratch root is hardcoded to /tmp/opencode. Those
          # bytes are pinned by the R4 binding, so provision the root inside
          # the derivation's private sandbox /tmp at integration level; no
          # reviewed source bytes change. Done here rather than in the
          # intake's packages.nix to keep the reviewed packet byte-exact.
          # The harness's fresh_root does a bare mkdir on
          # $out/share/ap5-qualification and requires a fresh directory whose
          # parent exists; the tools build only installs bin/, so provision
          # the share/ parent here the same way.
          spc-lab-ap5-qualified = ap5Packages.spc-lab-ap5-qualified.overrideAttrs (old: {
            preInstallCheck = (old.preInstallCheck or "") + ''
              mkdir -p /tmp/opencode
              mkdir -p "$out/share"
              # The Nix build sandbox sets HOME=/homeless-shelter (absent and
              # non-writable). chromium-156's crashpad handler aborts at
              # startup there ("--database is required", SIGTRAP) before the
              # DevTools endpoint comes up, so the ui-browser-tests gate step
              # would always hit its 15s Chrome timeout. chromium only needs
              # a writable HOME for its crash-database/config dirs; point it
              # at the derivation's private writable build top.
              export HOME="$NIX_BUILD_TOP"
            '';
            # Reproduce the reviewed installCheckPhase command exactly, plus
            # a failure-evidence capture: a failed derivation's $out is
            # deleted, which destroyed the gate's per-step output.log in
            # attempts 2/3 and left only the python traceback. On failure,
            # copy the gate root into the build directory so a
            # --keep-failed build preserves every step's captured output and
            # the browser step's proof/failure artifacts for post-mortem.
            # The success path is byte-identical to the reviewed phase; the
            # intake packages.nix stays byte-exact.
            installCheckPhase = ''
              runHook preInstallCheck
              gate_status=0
              ${pkgs.python3}/bin/python3 -B ${src}/testing/lab-ark-funded/portable.py gate \
                --source ${src} --installed "$out" --out "$out/share/ap5-qualification" \
                || gate_status=$?
              if [ "$gate_status" != 0 ]; then
                echo "gate failed (exit $gate_status); gate root preserved at $NIX_BUILD_TOP/gate-failure-evidence" >&2
                cp -a "$out/share/ap5-qualification" "$NIX_BUILD_TOP/gate-failure-evidence" || true
                exit "$gate_status"
              fi
              runHook postInstallCheck
            '';
          });
          spc-pool-ark = mkBinary "spc-pool" "rpc,ark";
          spc-agent-ark = mkBinary "spc-agent" "ark";
          spc-pool = mkBinary "spc-pool" "rpc";
          spc-agent = mkBinary "spc-agent" "";
          spc-cpu-miner = mkBinary "spc-cpu-miner" "";
          spc-lab-txd = mkBinary "spc-lab-txd" "";
          spc-scan = mkBinary "spc-scan" "";
          spc-lab = pkgs.rustPlatform.buildRustPackage (common // {
            pname = "spc-lab";
            version = "0.1.0";
            cargoBuildFlags = [ "-p" "spc-lab-cli" "--bin" "spc-lab" ];
            meta.mainProgram = "spc-lab";
          });
          default = mkBinary "spc-pool" "rpc";
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

          ui-node = pkgs.runCommand "spc-lab-ui-tests" { nativeBuildInputs = [ pkgs.nodejs ]; } ''
            cp -r ${src} source
            cd source
            node --test bins/spc-lab/tests/*.test.cjs > $out
          '';

          # Same-source native CPU / wallet joint component check. The hook
          # builds the CPU from this exact filtered source, generates the
          # immutable artifact descriptor from the real derivation, and
          # compiles the anchors into the wallet test before Cargo compiles
          # the test binary. No historical binary, archive or fixed temporary
          # path is an input; only SPC_NATIVE_JOINT_RESULTS is runtime-set.
          wallet-native-joint =
            let
              joint = import ./nix/checks/wallet-native-joint.nix {
                inherit pkgs common;
                cpu = self.packages.${system}.spc-cpu-miner;
              };
            in pkgs.rustPlatform.buildRustPackage (common // {
              pname = "wallet-native-joint";
              version = "0.1.0";
              doCheck = true;
              inherit (joint) preBuild preCheck;
              buildPhase = ''
                runHook preBuild
                cargo test --offline --locked -p spc-agent --features ark --lib --no-run
                runHook postBuild
              '';
              checkPhase = ''
                runHook preCheck
                ${pkgs.python3}/bin/python3 -B testing/native-joint/test_artifact.py
                cargo test --offline --locked -p spc-agent --features ark --lib \
                  wallet::native_audit::tests::joint_artifact::tests::
                cargo test --offline --locked -p spc-agent --features ark --lib \
                  wallet::native_audit::tests::sealed_native_cpu_waits_for_actual_wallet_collector_commit_before_first_hash \
                  -- --exact --nocapture
                runHook postCheck
              '';
              installPhase = joint.installResults;
            });

          # Normal ark feature tests: the joint test's ignore marker is
          # removed, so every derivation that compiles spc-agent ark lib
          # tests must carry the same anchors (compile-time) and a fresh
          # results directory (runtime), and preserve the receipt on install.
          # Scratch-root note: the reviewed R4 pool-control test module and
          # the frozen cpu-miner/agent test helpers hardcode the
          # /tmp/opencode scratch root (workstation convention; the lab
          # ark_infrastructure tests fall back to the platform temp dir but
          # these do not). The reviewed source bytes are frozen by the R4
          # binding, so this derivation provisions that root inside its own
          # private sandbox /tmp instead of changing reviewed source. No
          # host path is read; the sandbox /tmp is per-build.
          test-ark =
            let
              joint = import ./nix/checks/wallet-native-joint.nix {
                inherit pkgs common;
                cpu = self.packages.${system}.spc-cpu-miner;
              };
            in pkgs.rustPlatform.buildRustPackage (common // {
              pname = "spc-ark-feature-tests";
              version = "0.1.0";
              doCheck = true;
              # The two native_audit fixture-CPU tests spawn `python3` (the
              # reviewed test module's owned fixture interpreter), which is
              # not on the buildRustPackage sandbox PATH by default.
              nativeBuildInputs = common.nativeBuildInputs ++ [ pkgs.python3 ];
              inherit (joint) preBuild preCheck;
              preConfigure = "mkdir -p /tmp/opencode";
              installPhase = ''
                runHook preInstall
                mkdir -p "$out"
                ${joint.installResults}
                runHook postInstall
              '';
              checkPhase = ''
                runHook preCheck
                # --release matches every other consumer of these suites
                # (the assembler gate receipts, spc-lab-ark-tools' checkPhase
                # and the committed gate-plan.json rust steps). The frozen
                # wallet_control stall test asserts elapsed < 7s against the
                # 5s READ_DEADLINE; debug-profile execution of step()'s
                # durable control-dir writes does not fit that margin under
                # parallel test load, so a debug-profile check fails a
                # release-green suite.
                cargo test --release -p spc-crypto --features ark --offline
                cargo test --release -p spc-pool --features rpc,ark --offline
                cargo test --release -p spc-agent --features ark --offline
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
            pkgs.nodejs
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
