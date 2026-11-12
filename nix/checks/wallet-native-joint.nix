# Test-only hook. Primary attaches this to dedicated AND normal ark checks.
{ pkgs, common, cpu }:
let
  sourceId = common.SPC_BUILD_SOURCE_ID;
  artifact = pkgs.runCommand "native-joint-artifact" { } ''
    ${pkgs.python3}/bin/python3 ${../../testing/native-joint/artifact.py} \
      --cpu-output ${cpu} \
      --cpu-drv ${builtins.unsafeDiscardOutputDependency cpu.drvPath} \
      --source ${common.src} --source-id ${pkgs.lib.escapeShellArg sourceId} \
      --destination "$out" --nix-store ${pkgs.nix}/bin/nix-store
  '';
in {
  inherit artifact;
  # Run before any cargo invocation that could compile spc-agent tests.
  preBuild = ''
    . ${artifact}/anchors.sh
    export SPC_JOINT_TEST_SOURCE=${common.src}
    export SPC_BUILD_SOURCE_ID=${pkgs.lib.escapeShellArg sourceId}
    export CARGO_INCREMENTAL=0
  '';
  # Run before cargo test. Path is output-only, never artifact authority.
  preCheck = ''
    export SPC_NATIVE_JOINT_RESULTS="$(mktemp -d "$TMPDIR/native-joint.XXXXXXXX")"
    chmod 700 "$SPC_NATIVE_JOINT_RESULTS"
  '';
  # Append to installPhase; check outputs should be directories, not touch $out.
  installResults = ''
    test -n "$SPC_NATIVE_JOINT_RESULTS"
    test "$(find "$SPC_NATIVE_JOINT_RESULTS" -name receipt.json | wc -l)" -eq 1
    mkdir -p "$out"
    cp -R "$SPC_NATIVE_JOINT_RESULTS" "$out/native-joint"
    cp ${artifact}/descriptor.json "$out/native-joint-artifact.json"
  '';
}
