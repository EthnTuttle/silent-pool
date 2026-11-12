# Explicit lab artifact, not a replacement for the default L2 node pin.
# Exact tested AP1b + metadata snapshot, published in the owned rbitcoin fork.
# Fetch Git blobs: the bundled Bitcoin corpus has export-subst attributes
# which alter two files in archive exports. Local overrides keep the same NAR.
{ pkgs, nodeSource ? null }:
let
  sourceHash = "sha256-t6LQYTA738Uzstg5AsakWCJS4yGanDBTn4FSpUmhiIU=";
  frozen = builtins.fetchTree (if nodeSource == null then {
    type = "git";
    url = "https://github.com/EthnTuttle/rbitcoin.git";
    ref = "refs/heads/silent-pool-lab-profile-facts";
    rev = "c1505776117b1d4d42489f309af33978bbe124cc";
    shallow = true;
    submodules = false;
    narHash = sourceHash;
  } else {
    type = "path";
    path = nodeSource;
    narHash = sourceHash;
  });
  src = pkgs.lib.cleanSourceWith {
    src = frozen.outPath;
    filter = path: type: !(builtins.elem (baseNameOf path)
      [ ".git" "target" "__pycache__" ".cache" "node_modules" ]);
  };
  rust = pkgs.rust-bin.stable."1.95.0".default;
  build = pkgs.rustPlatform.buildRustPackage {
  pname = "rbitcoin-node-ark-lab";
  version = "0.8.99-profile-facts";
  inherit src;
  rustc = rust;
  cargo = rust;
  cargoLock = { lockFile = "${src}/Cargo.lock"; allowBuiltinFetchGit = true; };
  nativeBuildInputs = [ rust pkgs.pkg-config ];
  buildInputs = [ pkgs.openssl ];
  RUSTFLAGS = "--remap-path-prefix=/build/=/source/ -C debuginfo=0 -C strip=symbols";
  cargoBuildFlags = [ "-p" "rbitcoin-node" "--bin" "rbitcoin-node" ];
  doCheck = true;
  checkPhase = ''
    runHook preCheck
    # Upstream integration suites use debug-assertion-only fault hooks.
    # Keep the installed product release build separate from the test profile.
    cargo test --offline -p rbitcoin-rpc --test suite getblockchaininfo_
    cargo test --offline -p rbitcoin-node --test suite cli::
    runHook postCheck
  '';
  postInstall = ''
    mkdir -p $out/share/rbitcoin-lab
    cp Cargo.lock $out/share/rbitcoin-lab/Cargo.lock
    cat > $out/share/rbitcoin-lab/source.json <<'EOF'
    {"upstream_reference":"7736c8e779d9271b6a0c538881f9758527bc3165","ap1b_base_manifest":"da21f56dd6ab581e0d8353fe7431cc44efa9eb29bffb998459cbd7ac29105a9e","full_manifest":"f1592c935e29b0b1fdf6429ad1b84e9ae925e9a9fcccd66b04f3d090680ff20f","code_manifest":"9cbcb2a2051bd5da645872cc24d44b98048ce2b5892398d3f9b18f9611ad556d","metadata_patch":"f54df78cb69b6417e4ee9d07e0cfb175faf249a336d82a109ead929aebe25e7c","source_nar":"${sourceHash}","source_bytes":30897163,"rpc_profile":"explicit bark-v31 at runtime","funded_e2e":false}
    EOF
    $out/bin/rbitcoin-node --version > $out/share/rbitcoin-lab/version
  '';
  meta.mainProgram = "rbitcoin-node";
  };
in pkgs.runCommand "rbitcoin-node-ark-lab-provenance" {
  passthru.sourceBuild = build;
  meta.mainProgram = "rbitcoin-node";
} ''
  mkdir -p $out
  ln -s ${build}/bin $out/bin
  cp -r ${build}/share $out/share
  chmod u+w $out/share/rbitcoin-lab/version
  # The native node prints --version on stderr. Keep the tested release
  # artifact unchanged and capture both streams in its provenance layer.
  ${build}/bin/rbitcoin-node --version > $out/share/rbitcoin-lab/version 2>&1
  test -s $out/share/rbitcoin-lab/version
''
