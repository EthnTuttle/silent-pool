# Opt-in, Nix-store-linked Linux development node, not a service or launcher.
# The separate non-flake Git input and flake.lock pin must travel together.
{ pkgs, nodeSource }:
let
  pin = builtins.fromJSON (builtins.readFile ./testnet5/source.json);
  supported = builtins.elem pkgs.stdenv.hostPlatform.system [ "x86_64-linux" "aarch64-linux" ];
  src = pkgs.lib.cleanSourceWith {
    src = nodeSource.outPath;
    filter = path: type:
      let
        base = baseNameOf (toString path);
        rel = pkgs.lib.removePrefix (toString nodeSource.outPath + "/") (toString path);
        under = dir: rel == dir || pkgs.lib.hasPrefix (dir + "/") rel;
        noise = builtins.elem base [ ".git" "target" ".direnv" ".cache" "__pycache__"
          "node_modules" "datadir" "coverage" ".coverage" ".repro-out" "mutants.out" ]
          || pkgs.lib.hasPrefix "datadir-" base || pkgs.lib.hasPrefix "result" base
          || pkgs.lib.hasSuffix ".log" base
          || builtins.elem rel [ "fuzz/corpus" "fuzz/artifacts" ];
      in !noise && (builtins.elem rel [ "Cargo.toml" "Cargo.lock" "rust-toolchain.toml"
        ".cargo" ".cargo/config" ".cargo/config.toml" ] || under "crates" || under "fuzz");
  };
  rust = pkgs.rust-bin.stable."1.95.0".default;
in
assert supported;
assert nodeSource.rev == pin.revision;
assert nodeSource.narHash == pin.nar_hash;
assert builtins.hashFile "sha256" (nodeSource.outPath + "/Cargo.lock") == pin.cargo_lock_sha256;
pkgs.rustPlatform.buildRustPackage {
  pname = "rbitcoin-node-testnet5-dev";
  version = "0.9.99-8af95b8";
  inherit src;
  rustc = rust;
  cargo = rust;
  strictDeps = true;
  # This exact upstream lock has registry dependencies only. No guessed vendor
  # hash, unlocked cargo update, git dependency substitution or source patch.
  cargoLock.lockFile = nodeSource.outPath + "/Cargo.lock";
  nativeBuildInputs = [ rust pkgs.pkg-config pkgs.makeWrapper ];
  RUSTFLAGS = "--remap-path-prefix=/build/=/source/ -C debuginfo=0 -C strip=symbols";
  cargoBuildFlags = [ "-p" "rbitcoin-node" "-p" "rbitcoin-cli"
    "--bin" "rbitcoin-node" "--bin" "rbitcoin-cli" ];
  # Heavy consensus/CLI qualification is a separately scheduled recipe. A
  # product build must not launch nodes or silently claim those tests passed.
  doCheck = false;
  postInstall = ''
    mkdir -p "$out/share/testnet5-dev"
    cp ${./testnet5/source.json} "$out/share/testnet5-dev/source.json"
    cp ${./testnet5/network.json} "$out/share/testnet5-dev/network.json"
    cp ${./testnet5/node.conf.in} "$out/share/testnet5-dev/node.conf.in"
    cp ${./testnet5/validate-node.py} "$out/share/testnet5-dev/validate-node.py"
    cp ${nodeSource.outPath}/Cargo.lock "$out/share/testnet5-dev/Cargo.lock"
    makeWrapper ${pkgs.python3}/bin/python3 "$out/bin/testnet5-validate-node" \
      --add-flags "-B $out/share/testnet5-dev/validate-node.py"
  '';
  # Checksums MUST be taken after the fixup phase (strip/patchelf) has made
  # its final modifications to the installed executables; hashing in
  # postInstall records pre-fixup bytes that never match the store output.
  postFixup = ''
    sha256sum "$out/bin/rbitcoin-node" "$out/bin/rbitcoin-cli" \
      > "$out/share/testnet5-dev/binaries.sha256"
  '';
  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    "$out/bin/rbitcoin-node" --version > "$out/share/testnet5-dev/version.txt" 2>&1
    grep -F 'rbitcoin-node 0.9.99' "$out/share/testnet5-dev/version.txt"
    "$out/bin/rbitcoin-node" --help > "$TMPDIR/testnet5-node-help" 2>&1
    grep -F testnet5 "$TMPDIR/testnet5-node-help"
    ${pkgs.python3}/bin/python3 -B ${./testnet5}/test-validator.py
    # The recorded manifest must match the actual installed binaries as they
    # exist in the store output; a mismatch fails the build rather than
    # shipping a manifest that cannot be verified by a consumer.
    sha256sum --check "$out/share/testnet5-dev/binaries.sha256"
    runHook postInstallCheck
  '';
  passthru = {
    sourcePin = pin;
    network = builtins.fromJSON (builtins.readFile ./testnet5/network.json);
    qualification = "source-and-offline-fixture-only; build and live-node validation required";
  };
  meta = {
    description = "Pinned BIP95 Testnet5 development node and read-only identity validator";
    mainProgram = "rbitcoin-node";
    platforms = [ "x86_64-linux" "aarch64-linux" ];
    license = with pkgs.lib.licenses; [ mit asl20 ];
  };
}
