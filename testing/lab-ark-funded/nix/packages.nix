# Primary imports this from the pinned root flake; no local/private sources.
{ pkgs, rust, src, labTools, node, captaind, bark, probe, revision }:
assert pkgs.stdenv.hostPlatform.system == "x86_64-linux";
let
  harness = src + "/testing/lab-ark-funded";
  ap5-tools = pkgs.writeShellApplication {
    name = "ap5-tools";
    runtimeInputs = [ pkgs.nix pkgs.git ];
    text = ''
      command="''${1:?use portable, ap5, shared, or replay}"
      shift
      case "$command" in
        portable) entry=portable.py ;;
        ap5) entry=ap5.py ;;
        shared) entry=shared_run.py ;;
        replay) entry=verify_archive.py ;;
        *) echo "unknown AP5 entrypoint: $command" >&2; exit 2 ;;
      esac
      exec ${pkgs.python3}/bin/python3 -B ${harness}/"$entry" "$@"
    '';
  };
  ap5-vtxo-verifier = import ../verifier/package.nix { inherit pkgs rust; };
  ap5-qa-source = pkgs.runCommand "ap5-qa-source-r24" { } ''
    ${pkgs.python3}/bin/python3 -B ${harness}/portable.py package-qa --repo ${src} --out "$out"
  '';
  # test_probe_package copies the reference with shutil.copytree, which
  # preserves the store's read-only file modes, and then rewrites one
  # fixture to prove the runtime manifest does not hide the change. The
  # raw store path would fail that write with EACCES inside the Nix
  # installCheck too, so the test consumes a byte-identical writable
  # copy; ap5-inputs keeps binding the immutable original.
  ap5-probe-test-package = pkgs.runCommand "ap5-probe-test-package" { } ''
    cp -r ${probe} $out
    chmod -R u+w $out
  '';
  spc-lab-ap5-qualified =
    assert (builtins.fromJSON (builtins.readFile (src + "/testing/lab-ark-funded/source-contract.json"))).state == "source-bound-not-tested";
    labTools.overrideAttrs (old: {
    pname = "spc-lab-ap5-qualified";
    nativeBuildInputs = (old.nativeBuildInputs or []) ++ [ pkgs.python3 pkgs.nodejs pkgs.chromium ];
    CHROME = "${pkgs.chromium}/bin/chromium";
    SPC_LAB_RBITCOIN_NODE = "${node}/bin/rbitcoin-node";
    AP5_PROBE_TEST_PACKAGE = "${ap5-probe-test-package}";
    doInstallCheck = true;
    preBuild = (old.preBuild or "") + ''
      ${pkgs.python3}/bin/python3 -B ${harness}/portable.py check-contract --source ${src}
    '';
    installCheckPhase = ''
      runHook preInstallCheck
      ${pkgs.python3}/bin/python3 -B ${harness}/portable.py gate \
        --source ${src} --installed "$out" --out "$out/share/ap5-qualification"
      runHook postInstallCheck
    '';
  });
  runtime = import (src + "/nix/lab/ark-runtime.nix") {
    inherit pkgs src node captaind probe;
    lab = spc-lab-ap5-qualified; pool = spc-lab-ap5-qualified;
    miner = spc-lab-ap5-qualified; scanner = spc-lab-ap5-qualified;
    agent = spc-lab-ap5-qualified; txd = spc-lab-ap5-qualified;
  };
  ap5-inputs = pkgs.writeTextDir "outputs.json" (builtins.toJSON {
    schema = "spc.lab.ap5.nix-outputs/v1";
    base_revision = revision;
    binaries = {
      lab = "${spc-lab-ap5-qualified}/bin/spc-lab";
      txd = "${spc-lab-ap5-qualified}/bin/spc-lab-txd";
      pool = "${spc-lab-ap5-qualified}/bin/spc-pool";
      agent = "${spc-lab-ap5-qualified}/bin/spc-agent";
      miner = "${spc-lab-ap5-qualified}/bin/spc-cpu-miner";
      scanner = "${spc-lab-ap5-qualified}/bin/spc-scan";
      node = "${node}/bin/rbitcoin-node";
      captaind = "${captaind}/bin/captaind";
      bark = "${bark}/bin/bark";
      grpcurl = "${pkgs.grpcurl}/bin/grpcurl";
      probe = "${probe}/bin/ark-rpc-probe";
      redis-server = "${pkgs.redis}/bin/redis-server";
      redis-cli = "${pkgs.redis}/bin/redis-cli";
      psql = "${pkgs.postgresql_17}/bin/psql";
    };
    postgres_bin_dir = "${pkgs.postgresql_17}/bin";
    runtime_script = "${runtime}/bin/spc-lab-self-hosted-ark";
    pool_wallet_executor = "${spc-lab-ap5-qualified}/bin/pool-wallet-executor";
    vtxo_verifier = "${ap5-vtxo-verifier}/bin/ap5-vtxo-verifier";
    qa_descriptor = "${ap5-qa-source}/qa-source.json";
    qa_path_env = pkgs.lib.makeBinPath [ spc-lab-ap5-qualified node captaind bark probe pkgs.postgresql_17 pkgs.redis pkgs.grpcurl pkgs.coreutils ];
  });
in { inherit ap5-tools ap5-vtxo-verifier ap5-qa-source spc-lab-ap5-qualified ap5-inputs; }
