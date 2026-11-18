# Opt-in, Nix-store-linked Testnet5 explorer deployment, not a service or
# launcher daemon. Builds the rbitcoin.space explorer (smolcars/rbitcoin-space,
# MIT) from the pinned source in ./testnet5/explorer-source.json: the Rust
# server, its Solid/Vite frontend (built offline from the pinned bun lockfile),
# and a loopback launcher that points the explorer at the Testnet5 dev node's
# JSON-RPC, Esplora and Electrum endpoints. The node package itself
# (./testnet5-node.nix) is untouched: its rbitcoin-node binary already embeds
# the Esplora/Electrum servers; they are enabled per-run with
# --esplora-listen/--electrum-listen/--sh-index (see passthru.nodeFlags).
{ pkgs }:
let
  pin = builtins.fromJSON (builtins.readFile ./testnet5/explorer-source.json);
  supported = builtins.elem pkgs.stdenv.hostPlatform.system [ "x86_64-linux" "aarch64-linux" ];
  src = pkgs.fetchFromGitHub {
    owner = "smolcars";
    repo = "rbitcoin-space";
    rev = pin.revision;
    hash = pin.source_hash;
  };
  rust = pkgs.rust-bin.stable.${pin.rust_toolchain}.default;
  version = "${pin.workspace_version}-${builtins.substring 0 7 pin.revision}";

  # Frontend dependencies from the pinned bun lockfile, as a fixed-output
  # derivation: `bun install` needs the network once; the output is pinned.
  # --ignore-scripts: no lifecycle script may download or generate anything
  # (the vite build below runs from these packages as-is).
  webDeps = pkgs.stdenvNoCC.mkDerivation {
    pname = "rbitcoin-space-web-deps";
    inherit version src;
    nativeBuildInputs = [ pkgs.bun pkgs.cacert ];
    dontConfigure = true;
    buildPhase = ''
      runHook preBuild
      cd web
      export HOME="$TMPDIR"
      bun install --frozen-lockfile --ignore-scripts --no-progress
      runHook postBuild
    '';
    installPhase = ''
      runHook preInstall
      cp -r node_modules "$out"
      runHook postInstall
    '';
    dontFixup = true;
    outputHashMode = "recursive";
    outputHashAlgo = "sha256";
    outputHash = "sha256-cNYiP2OxaW6TcThds1gxRhv4RBktAiriAdBNYFh4GTY=";
  };

  web = pkgs.stdenvNoCC.mkDerivation {
    pname = "rbitcoin-space-web";
    inherit version src;
    nativeBuildInputs = [ pkgs.bun ];
    dontConfigure = true;
    buildPhase = ''
      runHook preBuild
      cd web
      export HOME="$TMPDIR"
      cp -r ${webDeps} node_modules
      chmod -R u+w node_modules
      bun node_modules/vite/bin/vite.js build
      runHook postBuild
    '';
    installPhase = ''
      runHook preInstall
      cp -r dist "$out"
      test -f "$out/index.html"
      test -f "$out/boot.js"
      runHook postInstall
    '';
  };

  server = pkgs.rustPlatform.buildRustPackage {
    pname = "rbitcoin-space-server";
    inherit version src;
    rustc = rust;
    cargo = rust;
    strictDeps = true;
    # Registry dependencies only at this pin (explorer-source.json); no vendor
    # hash guess, no cargo update, no git dependency substitution.
    cargoLock.lockFile = "${src}/Cargo.lock";
    nativeBuildInputs = [ rust ];
    RUSTFLAGS = "--remap-path-prefix=/build/=/source/ -C debuginfo=0 -C strip=symbols";
    cargoBuildFlags = [ "-p" "rbitcoin-space" "--bin" "rbitcoin-space" ];
    # Upstream's test suite is CI's job at the pin; a product build must not
    # silently claim it ran.
    doCheck = false;
  };

  # Node-side flags that make the dev node serve what the explorer and bark
  # wallets consume. --rpc-client-version 310000 is only needed when a bark
  # captaind shares the node (it refuses getnetworkinfo.version < 310000);
  # the explorer itself does not require it.
  nodeFlags = [
    "--sh-index"
    "--esplora-listen" "127.0.0.1:3003"
    "--electrum-listen" "127.0.0.1:50001"
    "--rpc-listen" "127.0.0.1:18352"
    "--rpc-cookie-file" "<datadir>/.cookie"
  ];
in
assert supported;
pkgs.stdenvNoCC.mkDerivation {
  pname = "rbitcoin-space-testnet5";
  inherit version;
  dontUnpack = true;
  nativeBuildInputs = [ pkgs.makeWrapper ];
  installPhase = ''
    runHook preInstall
    mkdir -p "$out/bin" "$out/share/rbitcoin-space" "$out/share/testnet5-dev"
    ln -s ${server}/bin/rbitcoin-space "$out/bin/rbitcoin-space"
    ln -s ${web} "$out/share/rbitcoin-space/web"
    cp ${./testnet5/explorer-source.json} "$out/share/testnet5-dev/explorer-source.json"

    # Loopback launcher: Testnet5 only, served at /testnet5 with
    # data-network="testnet5" (web/public/boot.js derives it from the path).
    # The six TESTNET5_* variables are the explorer's whole network config;
    # RPC credentials come from the node's Core-format cookie file.
    cat > "$out/bin/run-testnet5-explorer" <<'EOF'
    #!@shell@
    set -eu
    : "''${NODE_COOKIE_FILE:?set NODE_COOKIE_FILE to the dev node Core-format RPC cookie file (--rpc-cookie-file)}"
    rpc="''${TESTNET5_RPC:-127.0.0.1:18352}"
    cookie="$(cat "$NODE_COOKIE_FILE")"
    export TESTNET5_RPC_HOST="''${rpc%:*}"
    export TESTNET5_RPC_PORT="''${rpc##*:}"
    export TESTNET5_RPC_USER="''${cookie%%:*}"
    export TESTNET5_RPC_PASSWORD="''${cookie#*:}"
    export TESTNET5_ESPLORA_URL="''${TESTNET5_ESPLORA_URL:-http://127.0.0.1:3003}"
    export TESTNET5_ELECTRUM_ADDR="''${TESTNET5_ELECTRUM_ADDR:-127.0.0.1:50001}"
    export LISTEN_ADDR="''${LISTEN_ADDR:-127.0.0.1:8080}"
    export WEB_DIR="@out@/share/rbitcoin-space/web"
    # DATA_DIR (mining/mempool-history/audit store) passes through when set.
    exec "@out@/bin/rbitcoin-space" "$@"
    EOF
    sed -i -e "s|@out@|$out|g" -e "s|@shell@|${pkgs.runtimeShell}|g" \
      -e 's/^    //' "$out/bin/run-testnet5-explorer"
    chmod +x "$out/bin/run-testnet5-explorer"
    runHook postInstall
  '';
  doInstallCheck = true;
  installCheckPhase = ''
    runHook preInstallCheck
    test -x "$out/bin/rbitcoin-space"
    test -f "$out/share/rbitcoin-space/web/index.html"
    test -f "$out/share/rbitcoin-space/web/boot.js"
    grep -F 'dataset["network"]' "$out/share/rbitcoin-space/web/boot.js"
    runHook postInstallCheck
  '';
  passthru = {
    sourcePin = pin;
    inherit web webDeps server nodeFlags;
    qualification = pin.qualification;
  };
  meta = {
    description = "Pinned rbitcoin.space explorer (server + frontend) wired for the Testnet5 development node";
    mainProgram = "run-testnet5-explorer";
    platforms = [ "x86_64-linux" "aarch64-linux" ];
    license = pkgs.lib.licenses.mit;
  };
}
