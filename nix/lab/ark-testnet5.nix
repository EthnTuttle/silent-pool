{ pkgs, rust }:
let
  rev = "8c38e07ee70b046529a416b170f1ae5c77cc66a0";
  # The adopted testnet5 fork: stock c419159 plus network-identity-only
  # testnet5 support and a [patch.crates-io] bitcoin -> the adopted
  # rust-bitcoin fork (UPSTREAM_DEPS.md "Adopted forks (2026-11-17)";
  # decision 2026-11-17-bark-testnet5-fork.md). Opt-in packages only:
  # the lab's stock-server interop evidence stays on nix/lab/ark.nix.
  src = pkgs.fetchgit {
    url = "https://github.com/EthnTuttle/bark";
    inherit rev;
    hash = "sha256-5Cegrkc6pVBROAvPJd0hJDagXz7PznxKsMZtC04I1+o=";
  };
  mk = pname: package: binary: pkgs.rustPlatform.buildRustPackage {
    inherit pname src;
    version = "0.7.1-testnet5-${builtins.substring 0 7 rev}";
    rustc = rust;
    cargo = rust;
    # The fork's lock covers registry checksums and the exact git SHA of
    # the patched bitcoin fork; allowBuiltinFetchGit vendors git sources.
    cargoLock = {
      lockFile = "${src}/Cargo.lock";
      allowBuiltinFetchGit = true;
    };
    nativeBuildInputs = [ rust pkgs.pkg-config pkgs.protobuf ];
    buildInputs = [ pkgs.openssl pkgs.sqlite pkgs.postgresql_17 ];
    PROTOC = "${pkgs.protobuf}/bin/protoc";
    GIT_HASH = rev;
    SERVER_VERSION = "0.7.1-dev";
    BARK_VERSION = "0.7.1-dev";
    cargoBuildFlags = [ "-p" package "--bin" binary ]
      # Only the CLI is needed here, not barkd's downloadable Swagger UI.
      ++ pkgs.lib.optionals (package == "bark-cli")
        [ "--no-default-features" "--features" "tls-native-roots" ];
    doCheck = false; # Upstream integration tests require a funded regtest chain.
    postInstall = ''
      mkdir -p $out/share/ark
      cp -r server-rpc/protos $out/share/ark/
      cp server/captaind.default.toml $out/share/ark/
      cp Cargo.lock $out/share/ark/Cargo.lock
      printf '%s\n' '${rev}' > $out/share/ark/source-revision
      $out/bin/${binary} --version > $out/share/ark/version
    '';
    passthru = { inherit rev; upstreamSource = src; };
    meta.mainProgram = binary;
  };
in {
  captaind = mk "captaind-testnet5" "bark-server" "captaind";
  bark = mk "bark-testnet5" "bark-cli" "bark";
}
